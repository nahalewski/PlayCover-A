"""Read-only ARM64 IPA dyld metadata inspection; never executes app code."""
import collections
import hashlib
import json
import pathlib
import plistlib
import struct
import sys
import zipfile


def uleb(data, offset):
    value = 0
    for index in range(10):
        if offset >= len(data):
            raise ValueError('Truncated LEB operand')
        byte = data[offset]
        offset += 1
        value |= (byte & 127) << (index * 7)
        if not byte & 128:
            return value, offset
    raise ValueError('LEB operand exceeds 10 bytes')


def thin_arm64(data):
    architectures = []
    if data[:4] == bytes.fromhex('cafebabe'):
        selected = None
        for index in range(struct.unpack_from('>I', data, 4)[0]):
            cpu, subtype, offset, size, _ = struct.unpack_from('>5I', data, 8 + index * 20)
            architectures.append({'cpu_type': hex(cpu), 'cpu_subtype': subtype})
            if cpu == 0x100000c and selected is None:
                selected = data[offset:offset + size]
        if selected is None:
            raise ValueError('No ARM64 slice')
        data = selected
    if data[:4] != bytes.fromhex('cffaedfe') or struct.unpack_from('<I', data, 4)[0] != 0x100000c:
        raise ValueError('Expected little-endian ARM64 Mach-O')
    return data, architectures


def stream(data, kind, libraries):
    offset = 0
    opcodes = collections.Counter()
    symbol_flags = collections.Counter()
    types = set()
    references = set()
    nonweak = []
    ordinal = -3 if kind == 'weak' else 0
    name, flags, typ = '', 0, 1
    sites = 0
    threaded = False
    done = False
    while offset < len(data):
        byte = data[offset]
        offset += 1
        opcode, immediate = byte & 240, byte & 15
        opcodes[hex(opcode)] += 1
        if kind == 'rebase':
            if opcode == 0:
                done = True
                break
            if opcode == 0x10:
                types.add(immediate)
            elif opcode in (0x20, 0x30, 0x60, 0x70):
                _, offset = uleb(data, offset)
            elif opcode == 0x80:
                _, offset = uleb(data, offset)
                _, offset = uleb(data, offset)
            elif opcode not in (0x40, 0x50):
                raise ValueError(f'Unknown rebase opcode {opcode:#x}')
            continue
        if opcode == 0:
            done = True
            if kind != 'lazy':
                break
            ordinal, name, flags, typ = 0, '', 0, 1
        elif opcode == 0x10:
            ordinal = immediate
        elif opcode == 0x20:
            ordinal, offset = uleb(data, offset)
        elif opcode == 0x30:
            ordinal = immediate - 16 if immediate else 0
        elif opcode == 0x40:
            end = data.index(0, offset)
            name = data[offset:end].decode()
            offset = end + 1
            flags = immediate
            symbol_flags[hex(flags)] += 1
            if flags & 8:
                nonweak.append(name)
        elif opcode == 0x50:
            typ = immediate
            types.add(typ)
        elif opcode in (0x60, 0x70, 0x80):
            _, offset = uleb(data, offset)
        elif opcode in (0x90, 0xa0, 0xb0, 0xc0):
            count = 1
            if opcode == 0xa0:
                _, offset = uleb(data, offset)
            elif opcode == 0xc0:
                count, offset = uleb(data, offset)
                _, offset = uleb(data, offset)
            sites += count
            types.add(typ)
            references.add((ordinal, name, bool(flags & 1), typ))
        elif opcode == 0xd0:
            threaded = True
            if immediate == 0:
                _, offset = uleb(data, offset)
            elif immediate != 1:
                raise ValueError('Unknown threaded bind subopcode')
        else:
            raise ValueError(f'Unknown bind opcode {opcode:#x}')
    by_library = collections.Counter()
    for ordinal, _, _, _ in references:
        library = libraries[ordinal - 1]['name'] if 0 < ordinal <= len(libraries) else f'special:{ordinal}'
        by_library[library] += 1
    return {
        'opcode_counts': dict(sorted(opcodes.items())),
        'symbol_flag_counts': dict(sorted(symbol_flags.items())),
        'pointer_types': sorted(types),
        'threaded_binds': threaded,
        'contains_done_opcode': done,
        'binding_sites': sites,
        'unique_binding_references': len(references),
        'weak_import_references': sum(reference[2] for reference in references),
        'nonweak_definition_names': nonweak,
        'references_by_library': dict(sorted(by_library.items())),
    }


def selected_exports(trie, names):
    wanted = set(names)
    found = {}
    stack = [(0, '')]
    visited = set()
    while stack:
        offset, prefix = stack.pop()
        if (offset, prefix) in visited or len(visited) > 100000:
            raise ValueError('Cyclic or oversized selected export trie')
        visited.add((offset, prefix))
        size, position = uleb(trie, offset)
        end = position + size
        if end > len(trie):
            raise ValueError('Export terminal outside trie')
        if size and prefix in wanted:
            flags, cursor = uleb(trie, position)
            value, _ = uleb(trie, cursor)
            found[prefix] = {
                'export_flags': hex(flags),
                'weak_definition': bool(flags & 4),
                'reexport': bool(flags & 8),
                'kind': flags & 3,
                'value': hex(value),
                'strong_exported_definition': not bool(flags & (4 | 8)) and flags & 3 in (0, 2),
            }
        if end >= len(trie):
            raise ValueError('Missing export child count')
        count = trie[end]
        cursor = end + 1
        for _ in range(count):
            edge_end = trie.index(0, cursor)
            edge = trie[cursor:edge_end].decode()
            child, cursor = uleb(trie, edge_end + 1)
            child_prefix = prefix + edge
            if any(name.startswith(child_prefix) for name in wanted):
                stack.append((child, child_prefix))
    return {name: found.get(name, {'strong_exported_definition': False, 'absent': True}) for name in sorted(wanted)}


def inspect_image(path, original, cache_root):
    data, architectures = thin_arm64(original)
    libraries, regions, initializers, raw_streams = [], [], [], {}
    image = {'path': path, 'architecture': 'ARM64', 'size_bytes': len(data), 'fat_architectures': architectures,
             'sha256_arm64_slice': hashlib.sha256(data).hexdigest(), 'encrypted': False, 'dependencies': libraries,
             'segments': regions, 'initializer_sections': initializers, 'chained_fixups': False}
    offset = 32
    symtab = None
    command_end = offset + struct.unpack_from('<I', data, 20)[0]
    for _ in range(struct.unpack_from('<I', data, 16)[0]):
        command, size = struct.unpack_from('<II', data, offset)
        if size < 8 or offset + size > command_end:
            raise ValueError('Malformed load command')
        if command in (0xc, 0x80000018, 0x8000001f, 0x80000023, 0xd):
            name_offset = struct.unpack_from('<I', data, offset + 8)[0]
            name = data[offset + name_offset:offset + size].split(b'\0')[0].decode()
            if command == 0xd:
                image['install_name'] = name
            else:
                libraries.append({'ordinal': len(libraries) + 1, 'name': name, 'weak': command == 0x80000018,
                                  'reexport': command == 0x8000001f})
        elif command == 2:
            symtab = struct.unpack_from('<4I', data, offset + 8)
        elif command in (0x21, 0x2c):
            image['encrypted'] = bool(struct.unpack_from('<I', data, offset + 16)[0])
        elif command == 0x80000028:
            image['entry_main_offset'], image['entry_stack_size'] = struct.unpack_from('<QQ', data, offset + 8)
        elif command == 0x80000034:
            image['chained_fixups'] = True
        elif command == 0x19:
            name = data[offset + 8:offset + 24].split(b'\0')[0].decode()
            address, vmsize, fileoff, filesize = struct.unpack_from('<QQQQ', data, offset + 24)
            regions.append({'name': name, 'vmaddr': hex(address), 'vmsize': vmsize, 'fileoff': fileoff, 'filesize': filesize})
            for index in range(struct.unpack_from('<I', data, offset + 64)[0]):
                section = offset + 72 + index * 80
                name = data[section:section + 16].split(b'\0')[0].decode()
                flags = struct.unpack_from('<I', data, section + 64)[0]
                if flags & 255 in (9, 0x16) or name.startswith('__mod_init_func'):
                    address, section_size = struct.unpack_from('<QQ', data, section + 32)
                    width = 4 if flags & 255 == 0x16 else 8
                    initializers.append({'name': name, 'address': hex(address), 'size': section_size, 'count': section_size // width})
        elif command in (0x22, 0x80000022):
            for index, kind in enumerate(('rebase', 'bind', 'weak', 'lazy', 'exports')):
                raw_streams[kind] = struct.unpack_from('<II', data, offset + 8 + index * 8)
        offset += size
    decoded = {}
    for kind, (start, size) in raw_streams.items():
        if start + size > len(data):
            raise ValueError('Dyld stream outside image')
        decoded[kind] = {'offset': start, 'size': size}
        if kind != 'exports':
            decoded[kind].update(stream(data[start:start + size], kind, libraries))
    image['streams'] = decoded
    image['initializer_count'] = sum(section['count'] for section in initializers)
    names = decoded.get('weak', {}).get('nonweak_definition_names', [])
    if names:
        start, size = raw_streams['exports']
        image['nonweak_definition_export_audit'] = selected_exports(data[start:start + size], names)
        for entry in image['nonweak_definition_export_audit'].values():
            entry['symbol_table_matches'] = []
        if symtab:
            symbol_offset, count, string_offset, string_size = symtab
            if symbol_offset + count * 16 > len(data) or string_offset + string_size > len(data):
                raise ValueError('Symbol table outside image')
            strings = data[string_offset:string_offset + string_size]
            for index in range(count):
                name_offset, typ, section, description, value = struct.unpack_from('<IBBHQ', data, symbol_offset + index * 16)
                name = strings[name_offset:strings.index(0, name_offset)].decode()
                if name in image['nonweak_definition_export_audit']:
                    image['nonweak_definition_export_audit'][name]['symbol_table_matches'].append({
                        'type': hex(typ), 'section': section, 'description': hex(description), 'value': hex(value)})
        image['nonweak_definition_note'] = 'Flag 0x8 declarations name strong definitions but are not binding sites. No export address should be invented from a declaration alone.'
    image['missing_cache_paths'] = [library for library in libraries if library['name'].startswith('/')
                                    and not (cache_root / library['name'].lstrip('/')).is_file()]
    return image


ipa, cache_root, output = map(pathlib.Path, sys.argv[1:4])
digest = hashlib.sha256()
with ipa.open('rb') as source:
    for block in iter(lambda: source.read(1024 * 1024), b''):
        digest.update(block)
with zipfile.ZipFile(ipa) as archive:
    info_path = next(name for name in archive.namelist() if name.startswith('Payload/') and name.count('/') == 2 and name.endswith('/Info.plist'))
    info = plistlib.loads(archive.read(info_path))
    executable = info_path.removesuffix('Info.plist') + info['CFBundleExecutable']
    candidates = [executable]
    for name in archive.namelist():
        framework_binary = '.framework/' in name and name.split('.framework/', 1)[1] == name.split('.framework/', 1)[0].rsplit('/', 1)[-1]
        if name.endswith('.dylib') or framework_binary:
            candidates.append(name)
    images = [inspect_image(name, archive.read(name), cache_root) for name in candidates]
report = {
    'source_ipa': str(ipa), 'source_ipa_sha256': digest.hexdigest(), 'bundle_identifier': info['CFBundleIdentifier'],
    'app_version': info.get('CFBundleShortVersionString'), 'minimum_ios': info.get('MinimumOSVersion'),
    'architecture': 'ARM64', 'unencrypted': all(not image['encrypted'] for image in images),
    'execution_unsupported': True, 'scope': 'Read-only Mach-O metadata and path inventory; symbol availability and runtime execution are not verified.',
    'cache_inventory': str(cache_root), 'main_initializer_count': images[0]['initializer_count'], 'images': images,
}
output.write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
print(json.dumps({'report': str(output), 'source_sha256': digest.hexdigest(), 'main_nonweak_exports': images[0].get('nonweak_definition_export_audit', {})}, indent=2))
