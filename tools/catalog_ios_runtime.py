"""Inventory extracted ARM64 iOS libraries; this does not establish runnability."""
import argparse
import hashlib
import json
from pathlib import Path
import struct


def header(path):
    with path.open('rb') as stream:
        data = stream.read(32)
        if len(data) != 32 or struct.unpack_from('<I', data)[0] != 0xfeedfacf:
            raise ValueError('not a thin little-endian 64-bit Mach-O')
        _, cpu, subtype, kind, count, size, flags, _ = struct.unpack('<8I', data)
        if cpu != 0x100000c or kind != 6:
            raise ValueError(f'expected ARM64 MH_DYLIB, got cpu={cpu:#x}, type={kind}')
        if size > 16 * 1024 * 1024 or count > size // 8:
            raise ValueError('unreasonable load-command area')
        commands = stream.read(size)
        if len(commands) != size:
            raise ValueError('truncated load-command area')
    install_name = None
    offset = 0
    for _ in range(count):
        if offset + 8 > size:
            raise ValueError('truncated load command')
        command, length = struct.unpack_from('<II', commands, offset)
        if length < 8 or offset + length > size:
            raise ValueError('invalid load-command size')
        if command == 0xd:  # LC_ID_DYLIB
            if length < 24:
                raise ValueError('truncated LC_ID_DYLIB')
            name_offset = struct.unpack_from('<I', commands, offset + 8)[0]
            if not 24 <= name_offset < length:
                raise ValueError('invalid LC_ID_DYLIB name')
            name = commands[offset + name_offset:offset + length]
            if b'\0' not in name:
                raise ValueError('unterminated LC_ID_DYLIB name')
            install_name = name.split(b'\0', 1)[0].decode('utf-8')
        offset += length
    return {'magic': '0xfeedfacf', 'cpu_type': 'ARM64',
            'cpu_subtype': subtype, 'file_type': 'MH_DYLIB',
            'install_name': install_name, 'in_shared_cache': bool(flags & 0x80000000)}


def sha256(path):
    digest = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            digest.update(block)
    return digest.hexdigest()


def catalog(root, requirements):
    if not root.is_dir():
        raise ValueError(f'extracted-library root is not a directory: {root}')
    by_name, by_id, metadata, rejected = {}, {}, {}, {}
    for path in sorted(root.rglob('*')):
        if not path.is_file():
            continue
        by_name.setdefault(path.name, []).append(path)
        try:
            details = header(path)
        except (OSError, ValueError, UnicodeError, struct.error) as error:
            rejected[path] = str(error)
            continue
        metadata[path] = details
        if details['install_name']:
            by_id.setdefault(details['install_name'], []).append(path)
    weak = set(requirements.get('weak_libraries', []))
    required = set(requirements.get('required_libraries', []))
    entries = []
    for name in requirements['dynamic_libraries']:
        entry = {'requested_install_name': name,
                 'linkage': 'weak' if name in weak else 'required' if name in required else 'unknown'}
        if name.startswith('@'):
            entry.update(status='embedded_reference',
                         embedded_candidates=[item for item in requirements.get('embedded_dylibs', [])
                                              if item.endswith('/' + name.removeprefix('@rpath/'))] +
                                             [item + '/' + Path(name).name
                                              for item in requirements.get('embedded_frameworks', [])
                                              if item.endswith('/' + name.removeprefix('@rpath/').rsplit('/', 1)[0])])
            entries.append(entry)
            continue
        candidates = by_id.get(name, [])
        method = 'LC_ID_DYLIB'
        if not candidates:
            method = 'preserved_install_path'
            candidates = [p for p in by_name.get(Path(name).name, [])
                          if p.as_posix().endswith(name)]
        if not candidates:
            method = 'unique_basename'
            candidates = by_name.get(Path(name).name, [])
        if not candidates:
            entry['status'] = 'missing'
        elif len(candidates) != 1:
            entry.update(status='ambiguous', candidates=[str(p.resolve()) for p in candidates])
        else:
            path = candidates[0]
            entry.update(path=str(path.resolve()), size_bytes=path.stat().st_size,
                         sha256=sha256(path), match_method=method)
            if path not in metadata:
                entry.update(status='invalid', error=rejected[path])
            elif metadata[path]['install_name'] not in (None, name):
                entry.update(status='identity_mismatch', header=metadata[path])
            else:
                entry.update(status='found', header=metadata[path])
        entries.append(entry)
    return {'identifier': requirements.get('identifier'), 'root': str(root.resolve()),
            'scope': 'direct executable dependencies only; transitive closure not verified',
            'runtime_ready': False,
            'warning': 'Extracted dyld-cache images are inventory artifacts. Valid Mach-O headers do not prove independent loading, resolved fixups, or Android compatibility.',
            'summary': {status: sum(e['status'] == status for e in entries)
                        for status in sorted({e['status'] for e in entries})},
            'missing_required': [e['requested_install_name'] for e in entries
                                 if e['linkage'] == 'required' and e['status'] in
                                 ('missing', 'invalid', 'ambiguous', 'identity_mismatch')],
            'libraries': entries}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root', type=Path)
    parser.add_argument('report', type=Path, help='IPA runtime requirements JSON')
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    result = catalog(args.root, json.loads(args.report.read_text(encoding='utf-8-sig')))
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + '\n', encoding='utf-8')
    print(json.dumps({'output': str(args.output.resolve()), 'summary': result['summary'],
                      'missing_required': len(result['missing_required']), 'runtime_ready': False}))


if __name__ == '__main__':
    main()
