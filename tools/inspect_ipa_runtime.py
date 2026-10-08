"""List an IPA's native runtime requirements without executing its code."""
import json
import plistlib
import struct
import sys
import zipfile

with zipfile.ZipFile(sys.argv[1]) as archive:
    info_path = next(n for n in archive.namelist() if n.startswith('Payload/') and n.count('/') == 2 and n.endswith('.app/Info.plist'))
    info = plistlib.loads(archive.read(info_path))
    executable = info_path.removesuffix('Info.plist') + info['CFBundleExecutable']
    data = archive.read(executable)
    if data[:4] == bytes.fromhex('cafebabe'):
        count = struct.unpack_from('>I', data, 4)[0]
        for index in range(count):
            cpu, _, offset, size, _ = struct.unpack_from('>5I', data, 8 + 20 * index)
            if cpu == 0x100000c:
                data = data[offset:offset + size]
                break
    if struct.unpack_from('<I', data)[0] != 0xfeedfacf:
        raise ValueError('Expected ARM64 Mach-O')
    commands = struct.unpack_from('<I', data, 16)[0]
    libraries, features, required, weak = [], [], [], []
    offset = 32
    for _ in range(commands):
        command, size = struct.unpack_from('<II', data, offset)
        if command in (0xc, 0x18, 0x80000018, 0x8000001f, 0x80000023):
            name_offset = struct.unpack_from('<I', data, offset + 8)[0]
            library = data[offset + name_offset:offset + size].split(b'\0')[0].decode()
            libraries.append(library)
            (weak if command in (0x18, 0x80000018) else required).append(library)
        if command == 0x80000034: features.append('chained fixups')
        if command in (0x21, 0x2c) and struct.unpack_from('<I', data, offset + 16)[0]: features.append('encrypted executable')
        offset += size
    report = json.dumps({
        'identifier': info.get('CFBundleIdentifier'),
        'minimum_ios': info.get('MinimumOSVersion'),
        'executable': executable,
        'load_features': features,
        'dynamic_libraries': libraries,
        'required_libraries': required,
        'weak_libraries': weak,
        'embedded_frameworks': sorted(set(n.split('.framework/')[0] + '.framework' for n in archive.namelist() if '.framework/' in n)),
        'embedded_dylibs': sorted(n for n in archive.namelist() if n.startswith('Payload/') and n.endswith('.dylib')),
    }, indent=2)
    if len(sys.argv) == 3:
        from pathlib import Path
        Path(sys.argv[2]).write_text(report + '\n', encoding='utf-8')
        print('Saved library requirements to ' + sys.argv[2])
    else:
        print(report)
