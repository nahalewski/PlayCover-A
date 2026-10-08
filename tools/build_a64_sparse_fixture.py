"""Build a tiny ARM64 Mach-O whose data segment is eight GiB above its text."""
import pathlib
import struct

root = pathlib.Path(__file__).resolve().parent.parent / "touchHLE-src/tests/a64"


def segment(name, address, size, offset, file_size, protection):
    return struct.pack("<II16sQQQQIIII", 0x19, 72, name.encode(), address,
                       size, offset, file_size, protection, protection, 0, 0)


commands = segment("__TEXT", 0x100000000, 0x4000, 0, 0x4000, 5)
commands += segment("__DATA", 0x300000000, 0x4000, 0x4000, 8, 3)
commands += struct.pack("<IIQQ", 0x80000028, 24, 0x1000, 0)
header = struct.pack("<IIIIIIII", 0xFEEDFACF, 0x100000C, 0, 2, 3,
                     len(commands), 0, 0)
image = bytearray(0x4008)
image[:len(header + commands)] = header + commands
# movz x8,#3,lsl#32; mov x0,#42; str x0,[x8]; ldr x0,[x8]; ret
code = struct.pack("<5I", 0xD2C00068, 0xD2800540, 0xF9000100,
                   0xF9400100, 0xD65F03C0)
image[0x1000:0x1000 + len(code)] = code
(root / "sparse_client.macho").write_bytes(image)
print("Sparse ARM64 fixture generated: 8 GiB gap, 32 KiB image mappings")
