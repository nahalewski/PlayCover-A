"""Read-only bounded original15G77 ObjC image-reader dyld query callsites."""
import json
import struct
from pathlib import Path
from capstone import Cs, CS_ARCH_ARM64, CS_MODE_ARM

plan = json.loads(Path('ios-runtime/legacy-cache-loader-plan.json').read_text())
def read(address, size):
    region = next(r for r in plan['intervals'] if r['address'] <= address and address + size <= r['end_address'])
    with (Path(plan['cache_root']) / region['file']).open('rb') as stream:
        stream.seek(region['file_offset'] + address - region['address'])
        return stream.read(size)
decoder = Cs(CS_ARCH_ARM64, CS_MODE_ARM)
decoder.detail = True
results = []
# Audited _read_images entry through following timer/logging helper boundary.
start, end = 0x1800b4614, 0x1800b51b8
for instruction in decoder.disasm(read(start, end-start), start):
    if instruction.mnemonic != 'bl':
        continue
    target = instruction.operands[0].imm
    stub = list(decoder.disasm(read(target, 12), target))
    if len(stub) != 3 or [i.mnemonic for i in stub] != ['adrp', 'ldr', 'br']:
        continue
    slot = stub[0].operands[1].imm + stub[1].operands[1].mem.disp
    if slot not in [0x1ab7ab340, 0x1ab7ab3c8]:
        continue
    results.append({'call':hex(instruction.address), 'lr':hex(instruction.address+4),
                    'stub':hex(target), 'slot':hex(slot),
                    'query':'is_memory_immutable' if slot == 0x1ab7ab340 else 'program_sdk_version'})
result = {'scope':'Original15G77 bounded _read_images disassembly only; no execution or readiness',
          'start':hex(start), 'end':hex(end), 'calls':results}
Path('ios-runtime/legacy-objc-query-calls-audit.json').write_text(json.dumps(result, indent=2)+'\n')
print(json.dumps(result))
