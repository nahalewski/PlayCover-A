"""Index IPA metadata without extracting executables or importing onto devices."""
from pathlib import Path
import json
import plistlib
import struct
import zipfile

library = Path(r'C:\Users\Ben\Desktop\ipa')
output = Path('ios-runtime')
output.mkdir(exist_ok=True)
records = []
for path in sorted(library.glob('*.ipa'), key=lambda p: p.name.lower()):
    item = dict(path=str(path), filename=path.name, bytes=path.stat().st_size,
                modified_ns=path.stat().st_mtime_ns)
    try:
        with zipfile.ZipFile(path) as archive:
            plist_path = next(n for n in archive.namelist() if n.startswith('Payload/') and n.count('/') == 2 and n.endswith('/Info.plist'))
            info = plistlib.loads(archive.read(plist_path))
            name = info.get('CFBundleDisplayName') or info.get('CFBundleName') or path.stem
            executable = plist_path.rsplit('/', 1)[0] + '/' + info['CFBundleExecutable']
            with archive.open(executable) as stream:
                header = stream.read(4096)
            slices = []
            if header[:4] == bytes.fromhex('cafebabe'):
                count = struct.unpack_from('>I', header, 4)[0]
                assert 0 < count <= 32 and 8 + count * 20 <= len(header)
                for i in range(count):
                    cpu, subtype, offset, size, align = struct.unpack_from('>IIIII', header, 8 + i * 20)
                    slices.append(dict(cpu=cpu, subtype=subtype, offset=offset, bytes=size))
            elif header[:4] in (bytes.fromhex('cffaedfe'), bytes.fromhex('cefaedfe')):
                cpu, subtype = struct.unpack_from('<II', header, 4)
                slices.append(dict(cpu=cpu, subtype=subtype, offset=0, bytes=archive.getinfo(executable).file_size))
            else:
                raise ValueError('Unsupported executable header')
            architectures = []
            for thin in slices:
                cpu = thin['cpu']
                architecture = 'ARM64' if cpu == 0x100000c else 'ARM32' if cpu == 12 else f'CPU {cpu:#x}'
                if architecture not in architectures:
                    architectures.append(architecture)
                # Check command metadata only near the beginning of the ZIP
                # stream. Seeking deep FAT slices would decompress gigabytes.
                thin['cryptid'] = None
                thin['encryption_checked'] = False
                if thin['offset'] <= 1024 * 1024:
                    with archive.open(executable) as stream:
                        stream.seek(thin['offset'])
                        h = stream.read(32)
                        command_count, command_bytes = struct.unpack_from('<II', h, 16)
                        assert command_count <= 4096 and command_bytes <= 1024 * 1024
                        stream.seek(thin['offset'] + (32 if cpu == 0x100000c else 28))
                        commands = stream.read(command_bytes)
                    pos = 0
                    for _ in range(command_count):
                        cmd, length = struct.unpack_from('<II', commands, pos)
                        assert length >= 8 and pos + length <= len(commands)
                        if cmd in (0x21, 0x2c):
                            assert length >= 20
                            thin['cryptid'] = struct.unpack_from('<I', commands, pos + 16)[0]
                        pos += length
                    thin['encryption_checked'] = True
            item.update(display_name=name, bundle=info.get('CFBundleIdentifier'),
                        version=info.get('CFBundleShortVersionString'), minimum_ios=info.get('MinimumOSVersion'),
                        architectures=architectures, slices=slices,
                        capabilities=info.get('UIRequiredDeviceCapabilities', []))
    except Exception as error:
        item['error'] = str(error)
    records.append(item)
destination = output / 'IPA_LIBRARY_INDEX.json'
destination.write_text(json.dumps(dict(library=str(library), items=records), indent=2, ensure_ascii=False), encoding='utf-8')
def cell(value):
    return str(value or 'Unknown').replace('|', '\\|').replace('\n', ' ')
lines = ['# Backed-up IPA library', '', f'Source: `{library}`.', '',
         'Metadata index only; files were not modified or imported. Architecture is read from Mach-O headers.',
         'Presence of ARM64 does not mean the game runs. Deep FAT-slice encryption commands are intentionally not rescanned.', '',
         '| App | Version | Architecture | Minimum iOS | IPA |', '| --- | --- | --- | --- | --- |']
for item in records:
    lines.append('| ' + ' | '.join(cell(value) for value in (item.get('display_name'), item.get('version'),
        ' + '.join(item.get('architectures', [])), item.get('minimum_ios'), item['filename'])) + ' |')
lines += ['', f'Total: {len(records)} IPAs. Exact paths, sizes, modified times and checked cryptids are in `IPA_LIBRARY_INDEX.json`.',
          'Rescan only files whose size/modified time changed or when a targeted test requires deeper inspection.']
(output / 'IPA_LIBRARY_INDEX.md').write_text('\n'.join(lines) + '\n', encoding='utf-8')
print(json.dumps(dict(count=len(records), arm64=sum('ARM64' in r.get('architectures', []) for r in records),
    arm64_only=sum(r.get('architectures') == ['ARM64'] for r in records), errors=[r['filename'] for r in records if 'error' in r])))
