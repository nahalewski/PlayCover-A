#!/usr/bin/env python3
"""Bounded original-cache initializer inventory and ARM64 disassembly."""
import argparse
import json
from pathlib import Path
import struct
from capstone import Cs, CS_ARCH_ARM64, CS_MODE_ARM


def audit(plan_path, paths, function_targets=(), data_targets=()):
    plan = json.loads(plan_path.read_text())
    root = Path(plan['cache_root'])
    def region(address, size):
        return next(r for r in plan['intervals'] if r['address'] <= address and address + size <= r['end_address'])
    def read(address, size):
        if size > 1024 * 1024:
            raise ValueError('Oversized audit read')
        r = region(address, size)
        with (root / r['file']).open('rb') as stream:
            stream.seek(r['file_offset'] + address - r['address'])
            data = stream.read(size)
        if len(data) != size:
            raise ValueError('Short cache read')
        return data
    def pointer(address):
        raw = struct.unpack('<Q', read(address, 8))[0]
        slide = region(address, 8).get('slide_info')
        if not slide:
            return raw
        mask = int(slide['delta_mask'], 0) if isinstance(slide['delta_mask'], str) else slide['delta_mask']
        value = raw & ~mask
        return value + slide['value_add'] if value else 0
    result = []
    decoder = Cs(CS_ARCH_ARM64, CS_MODE_ARM)
    decoder.detail = True
    def first_call_chain(target):
        chain = []
        seen = set()
        for _ in range(8):
            if target in seen:
                break
            seen.add(target)
            code = list(decoder.disasm(read(target, 512), target))
            chain.append({'target': hex(target), 'instructions': [{'address': hex(i.address), 'mnemonic': i.mnemonic, 'operands': i.op_str} for i in code]})
            if len(code) >= 3 and code[0].mnemonic == 'adrp' and code[1].mnemonic == 'add' and code[2].mnemonic == 'br' and code[0].reg_name(code[0].operands[0].reg) == 'x16' and code[1].reg_name(code[1].operands[0].reg) == 'x16' and code[1].reg_name(code[1].operands[1].reg) == 'x16' and code[2].reg_name(code[2].operands[0].reg) == 'x16':
                target = code[0].operands[1].imm + code[1].operands[2].imm
                continue
            # Exact standard ARM64 shared-cache stub: ADRP x16; LDR x16; BR x16.
            if len(code) >= 3 and code[0].mnemonic == 'adrp' and code[1].mnemonic == 'ldr' and code[2].mnemonic == 'br' and code[0].reg_name(code[0].operands[0].reg) == 'x16' and code[1].reg_name(code[1].operands[0].reg) == 'x16' and code[1].reg_name(code[1].operands[1].mem.base) == 'x16' and code[2].reg_name(code[2].operands[0].reg) == 'x16':
                slot = code[0].operands[1].imm + code[1].operands[1].mem.disp
                target = pointer(slot)
                chain[-1]['stub_slot'] = hex(slot)
                continue
            call = next((i for i in code if i.mnemonic == 'bl'), None)
            if call is None:
                break
            target = call.operands[0].imm
        return chain
    for path in paths:
        image = next(i for i in plan['images'] if i['path'] == path)
        header = read(image['address'], 32)
        magic, cpu, subtype, kind, count, size, flags, reserved = struct.unpack('<8I', header)
        if magic != 0xfeedfacf or cpu != 0x100000c or count > 4096:
            raise ValueError('Invalid ARM64 header')
        commands = read(image['address'] + 32, size)
        cursor, initializers, section_inventory = 0, [], []
        for _ in range(count):
            command, length = struct.unpack_from('<II', commands, cursor)
            if length < 8 or cursor + length > len(commands):
                raise ValueError('Invalid command bounds')
            if command == 25:
                sections = struct.unpack_from('<I', commands, cursor + 64)[0]
                if 72 + sections * 80 > length:
                    raise ValueError('Invalid section table')
                for index in range(sections):
                    base = cursor + 72 + index * 80
                    name, segment, address, length_bytes = struct.unpack_from('<16s16sQQ', commands, base)
                    section_flags = struct.unpack_from('<I', commands, base + 64)[0]
                    section_inventory.append({'name': name.rstrip(b'\0').decode(), 'type': hex(section_flags & 0xff), 'address': hex(address), 'bytes': length_bytes})
                    if section_flags & 0xff in (9, 0x16):
                        stride = 8 if section_flags & 0xff == 9 else 4
                        if length_bytes % stride or length_bytes > 8192:
                            raise ValueError('Invalid initializer list')
                        for offset in range(0, length_bytes, stride):
                            target = pointer(address + offset) if stride == 8 else image['address'] + struct.unpack('<I', read(address + offset, 4))[0]
                            instructions = [{'address': hex(i.address), 'mnemonic': i.mnemonic, 'operands': i.op_str}
                                            for i in decoder.disasm(read(target, 512), target)]
                            initializers.append({'slot': hex(address + offset), 'target': hex(target), 'instructions': instructions, 'bounded_first_call_chain': first_call_chain(target)})
            cursor += length
        result.append({'path': path, 'header_address': image['address_hex'], 'sections': section_inventory, 'initializers': initializers})
    return {'scope': 'Read-only original cache; first 512 bytes per initializer. No guest execution.',
            'cache_plan': str(plan_path), 'images': result,
            'explicit_function_chains': [first_call_chain(target) for target in function_targets],
            'explicit_data': [{'address': hex(target), 'bytes': read(target, 48).hex(),
                               'cstring': read(target, 48).split(b'\0')[0].decode('utf-8', errors='replace')}
                              for target in data_targets]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--plan', type=Path, default=Path('ios-runtime/cache-loader-plan.json'))
    parser.add_argument('--image', action='append', default=[])
    parser.add_argument('--function', action='append', type=lambda value: int(value, 0), default=[])
    parser.add_argument('--data', action='append', type=lambda value: int(value, 0), default=[])
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    report = audit(args.plan, args.image, args.function, args.data)
    args.output.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps([{'path': i['path'], 'initializer_targets': [v['target'] for v in i['initializers']]} for i in report['images']], indent=2))


if __name__ == '__main__':
    main()
