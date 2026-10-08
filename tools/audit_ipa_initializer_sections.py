"""Read-only ordinary IPA initializer table metadata; no guest execution."""
import sys, zipfile, plistlib, struct

with zipfile.ZipFile(sys.argv[1]) as archive:
    plist = next(n for n in archive.namelist() if n.count('/') == 2 and n.endswith('/Info.plist'))
    info = plistlib.loads(archive.read(plist))
    path = plist.rsplit('/', 1)[0] + '/' + info['CFBundleExecutable']
    data = archive.read(path)
print(path, 'magic', hex(struct.unpack_from('<I', data)[0]))
if struct.unpack_from('<I', data)[0] != 0xfeedfacf:
    raise SystemExit('This bounded audit requires thin little-endian Mach-O64')
offset = 32
for _ in range(struct.unpack_from('<I', data, 16)[0]):
    command, command_size = struct.unpack_from('<II', data, offset)
    if command_size < 8 or offset + command_size > len(data):
        raise SystemExit('invalid command bounds')
    if command == 0x19:
        segment = data[offset+8:offset+24].rstrip(b'\0').decode()
        count = struct.unpack_from('<I', data, offset+64)[0]
        if 72+count*80 > command_size:
            raise SystemExit('invalid section bounds')
        for index in range(count):
            section = offset+72+index*80
            name = data[section:section+16].rstrip(b'\0').decode()
            address, size = struct.unpack_from('<QQ', data, section+32)
            flags = struct.unpack_from('<I', data, section+64)[0]
            if flags & 255 in (9, 0x16) or name.startswith('__mod_init_func'):
                width = 4 if flags & 255 == 0x16 else 8
                print(segment, name, f'address={address:#x} size={size} flags={flags:#x}',
                      f'width={width} count={size//width} address_remainder={address%width} size_remainder={size%width}')
    offset += command_size
