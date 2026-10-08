import hashlib
import json
from pathlib import Path
import struct
import zipfile

source = Path(r'C:\Users\Ben\Downloads\Terraria_4.5.0.ipa')
images = []
with zipfile.ZipFile(source) as archive:
    names = [n for n in archive.namelist() if n.endswith('.dylib') or
             ('.framework/' in n and n.split('.framework/')[1] == n.split('.framework/')[0].rsplit('/', 1)[-1])]
    names.insert(0, 'Payload/Terraria.app/Terraria')
    for name in names:
        data = archive.read(name)
        if data[:4] == bytes.fromhex('cafebabe'):
            architectures = []
            for index in range(struct.unpack_from('>I', data, 4)[0]):
                cpu, subtype, offset, size, _ = struct.unpack_from('>5I', data, 8 + index * 20)
                architectures.append([cpu, subtype])
                if cpu == 0x100000c:
                    selected = data[offset:offset + size]
            data = selected
        else:
            architectures = [list(struct.unpack_from('<II', data, 4))]
        if struct.unpack_from('<I', data)[0] != 0xfeedfacf:
            raise ValueError('Expected ARM64 Mach-O')
        offset = 32
        commands, dependencies, encryption, streams = [], [], [], {}
        entry = None
        initializer_count = 0
        for _ in range(struct.unpack_from('<I', data, 16)[0]):
            command, size = struct.unpack_from('<II', data, offset)
            commands.append(hex(command))
            if command in (0x21, 0x2c):
                encryption.append(dict(zip(('cryptoff', 'cryptsize', 'cryptid'), struct.unpack_from('<III', data, offset + 8))))
            if command in (0xc, 0x80000018, 0x8000001f):
                start = struct.unpack_from('<I', data, offset + 8)[0]
                dependencies.append({'path': data[offset + start:offset + size].split(b'\0')[0].decode(), 'weak': command == 0x80000018})
            if command == 0x80000028:
                entry = struct.unpack_from('<Q', data, offset + 8)[0]
            if command in (0x22, 0x80000022):
                streams = dict(zip(('rebase', 'bind', 'weak_bind', 'lazy_bind', 'exports'), struct.unpack_from('<10I', data, offset + 8)[1::2]))
            if command == 0x19:
                for index in range(struct.unpack_from('<I', data, offset + 64)[0]):
                    section = offset + 72 + index * 80
                    flags = struct.unpack_from('<I', data, section + 64)[0] & 255
                    if flags in (9, 0x16):
                        initializer_count += struct.unpack_from('<Q', data, section + 40)[0] // (8 if flags == 9 else 4)
            offset += size
        images.append({'path': name, 'size': len(data), 'architectures': architectures,
                       'encryption': encryption, 'commands': commands,
                       'dependencies': dependencies, 'legacy_stream_sizes': streams,
                       'entry_offset': entry, 'initializer_count': initializer_count})
report = {'source': str(source), 'sha256': hashlib.file_digest(source.open('rb'), 'sha256').hexdigest(),
          'images': images, 'scope': 'Static metadata only; no app execution or decryption performed.'}
Path('Terraria-native-images.json').write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps(report, indent=2))
