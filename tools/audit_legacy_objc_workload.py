"""Read-only selected dependency closure and ObjC section workload inventory.

Uses previously verified thin consumer metadata and original cache mappings.
No names/strings from runtime process memory, no execution or budget change.
"""
import json
import struct
from pathlib import Path

plan = json.loads(Path('ios-runtime/legacy-cache-loader-plan.json').read_text())
consumers = json.loads(Path('ios-runtime/legacy-cpp-consumer-audit.json').read_text())
quest = next(record for record in consumers if 'quest' in record['ipa'].lower())
catalogue = {record['path']: record for record in plan['images']}

def read(address, size):
    assert size <= 1024 * 1024
    interval = next(r for r in plan['intervals'] if r['address'] <= address and address + size <= r['end_address'])
    with (Path(plan['cache_root']) / interval['file']).open('rb') as source:
        source.seek(interval['file_offset'] + address - interval['address'])
        data = source.read(size)
    assert len(data) == size
    return data

def metadata(image):
    header = read(image['address'], 32)
    magic, cpu, _, _, count, size, flags, _ = struct.unpack('<8I', header)
    assert magic == 0xfeedfacf and cpu == 0x100000c and count <= 4096
    commands = read(image['address'] + 32, size)
    position = 0
    dependencies, sections = [], []
    for _ in range(count):
        command, length = struct.unpack_from('<II', commands, position)
        assert length >= 8 and position + length <= len(commands)
        body = commands[position:position + length]
        if command in (0xc, 0x80000018, 0x8000001f, 0x80000023):
            offset = struct.unpack_from('<I', body, 8)[0]
            dependencies.append(body[offset:].split(b'\0')[0].decode())
        if command == 0x19:
            n = struct.unpack_from('<I', body, 64)[0]
            assert length == 72 + n * 80
            for at in range(72, length, 80):
                name, segment, address, size = struct.unpack_from('<16s16sQQ', body, at)
                sections.append((name.rstrip(b'\0').decode(), segment.rstrip(b'\0').decode(), size))
        position += length
    assert position == len(commands)
    return flags, dependencies, sections

todo = [d['name'] for image in quest['images'] for d in image['dependencies'] if d['name'] in catalogue]
selected = {}
while todo:
    path = todo.pop()
    if path in selected:
        continue
    flags, dependencies, sections = metadata(catalogue[path])
    selected[path] = (flags, sections)
    todo.extend(d for d in dependencies if d in catalogue)

totals = {}
objc_images = 0
for flags, sections in selected.values():
    eligible = flags & 0x40000000 and any(name == '__objc_imageinfo' and segment.startswith('__DATA') for name, segment, _ in sections)
    if not eligible:
        continue
    objc_images += 1
    for name, _, size in sections:
        if name in ('__objc_classlist', '__objc_protolist', '__objc_protorefs', '__objc_catlist', '__objc_selrefs', '__objc_classrefs', '__objc_superrefs', '__objc_nlclslist', '__objc_nlcatlist'):
            assert size % 8 == 0
            totals[name] = totals.get(name, 0) + size // 8

output = {'scope': 'Read-only original cache selected dependency closure, reconstructed from verified consumer dependencies; no runtime-ready receipt or rigorous instruction upper bound',
          'cache_uuid': plan['cache_uuid'], 'cache_selected': len(selected), 'cache_objc_images': objc_images,
          'ordinary_images': len(quest['images']), 'selected_cache_section_pointer_counts': totals,
          'tick_contract': 'Dynarmic frontend increments CycleCount once per translated guest instruction; backend charges block.CycleCount, not one tick per basic block. Block-granular exhaustion can overshoot a slice by a block.',
          'limitations': 'Ordinary image section counts not included; no dynamic duplicate-class/protocol/name-length or hash-chain upper bound inferred from table size alone.'}
Path('ios-runtime/legacy-objc-workload-533.json').write_text(json.dumps(output, indent=2) + '\n')
print(json.dumps(output, indent=2))
