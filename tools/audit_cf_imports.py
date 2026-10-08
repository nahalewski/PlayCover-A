"""List direct undefined CF C symbols in an IPA's thin ARM64 Mach-O files."""
import json
from pathlib import Path
import struct
import sys
import zipfile

result = {}
with zipfile.ZipFile(sys.argv[1]) as archive:
    for name in archive.namelist():
        if not name.startswith('Payload/') or name.endswith('/'):
            continue
        # Only likely executable/framework/dylib filenames; skip game assets.
        if '.' in name.rsplit('/', 1)[-1] and not name.endswith('.dylib'):
            continue
        with archive.open(name) as stream:
            header = stream.read(32)
        if len(header) < 32 or struct.unpack_from('<I', header)[0] != 0xfeedfacf:
            continue
        if struct.unpack_from('<I', header, 4)[0] != 0x100000c:
            continue
        data = archive.read(name)
        offset = 32
        symbols = []
        for _ in range(struct.unpack_from('<I', data, 16)[0]):
            command, size = struct.unpack_from('<II', data, offset)
            if size < 8 or offset + size > len(data):
                raise ValueError('Invalid Mach-O command bounds')
            if command == 2:
                sym, count, strings, length = struct.unpack_from('<IIII', data, offset + 8)
                if sym + count * 16 > len(data) or strings + length > len(data):
                    raise ValueError('Invalid symbol table bounds')
                for i in range(count):
                    index, kind, section, description, value = struct.unpack_from('<IBBHQ', data, sym + i * 16)
                    if index >= length:
                        raise ValueError('Invalid symbol name bounds')
                    symbol = data[strings + index:strings + length].split(b'\0', 1)[0].decode(errors='replace')
                    if kind & 14 == 0 and symbol.startswith('_CF'):
                        symbols.append(symbol)
            offset += size
        result[name] = sorted(set(symbols))
report = dict(source=sys.argv[1], scope='Direct undefined CF symbols only; no execution, thin ARM64 only', binaries=result)
Path(sys.argv[2]).write_text(json.dumps(report, indent=2), encoding='utf-8')
print(json.dumps(report, indent=2))
