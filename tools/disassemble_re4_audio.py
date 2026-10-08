"""Inspect original RE4 audio callback code in the user's unmodified IPA."""
import struct
import zipfile
from capstone import Cs, CS_ARCH_ARM, CS_MODE_ARM

data = zipfile.ZipFile('pixel-fold-tests/ResidentEvil4-original.ipa').read('Payload/Res4-iPad.app/Res4-iPad')
count = struct.unpack_from('<I', data, 16)[0]
position = 28
segments = []
for _ in range(count):
    command, size = struct.unpack_from('<II', data, position)
    if command == 1:
        name, address, length, offset, disk_size = struct.unpack_from('<16sIIII', data, position + 8)
        segments.append((address, length, offset, disk_size))
    position += size
decoder = Cs(CS_ARCH_ARM, CS_MODE_ARM)
for start, length in [(0x84534, 0x180), (0x83c78, 0x100), (0x84b64, 0x140)]:
    segment = next(item for item in segments if item[0] <= start < item[0] + item[1])
    offset = start - segment[0] + segment[2]
    print(f'FUNCTION {start:#x}')
    for instruction in decoder.disasm(data[offset:offset+length], start):
        print(f'{instruction.address:#x}: {instruction.mnemonic} {instruction.op_str}')
