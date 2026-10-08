"""Read-only ARMv7 class/selector and keyed-NIB inventory for the user's IPA."""
import collections
import hashlib
import json
from pathlib import Path
import plistlib
import re
import struct
import zipfile

ipa = Path('PokedexPlus-device.ipa')
report = {'source': str(ipa), 'sha256': hashlib.sha256(ipa.read_bytes()).hexdigest(), 'nibs': {}}
with zipfile.ZipFile(ipa) as archive:
    info = plistlib.loads(archive.read('Payload/PokedexPlus.app/Info.plist'))
    report['bundle'] = info
    raw = archive.read('Payload/PokedexPlus.app/' + info['CFBundleExecutable'])
    if raw[:4] == bytes.fromhex('cafebabe'):
        for index in range(struct.unpack_from('>I', raw, 4)[0]):
            cpu, subtype, offset, size, _ = struct.unpack_from('>5I', raw, 8 + index * 20)
            if cpu == 12 and subtype & 0xffffff == 9:
                raw = raw[offset:offset + size]
                break
        else:
            raise ValueError('No ARMv7 slice')
    assert struct.unpack_from('<I', raw)[0] == 0xfeedface
    report['architecture'] = 'ARMv7'
    offset = 28
    sections = {}
    for _ in range(struct.unpack_from('<I', raw, 16)[0]):
        cmd, size = struct.unpack_from('<II', raw, offset)
        if cmd == 1:
            for index in range(struct.unpack_from('<I', raw, offset + 48)[0]):
                section = offset + 56 + index * 68
                name = raw[section:section + 16].split(b'\0')[0].decode()
                address, length, fileoff = struct.unpack_from('<III', raw, section + 32)
                sections[name] = (address, length, fileoff)
        elif cmd == 2:
            symoff, count, stroff, strlen = struct.unpack_from('<4I', raw, offset + 8)
            symbols = []
            strings = raw[stroff:stroff + strlen]
            for index in range(count):
                strx, ntype, _, _, nvalue = struct.unpack_from('<IBBHI', raw, symoff + index * 12)
                if ntype & 0x0e == 0 and nvalue == 0:
                    symbols.append(strings[strx:strings.index(0, strx)].decode(errors='replace'))
            report['objc_import_classes'] = sorted({n.split('$_', 1)[1] for n in symbols if n.startswith('_OBJC_CLASS_$_')})
        offset += size
    for section, key in [('__objc_methname', 'selectors'), ('__objc_classname', 'binary_classnames')]:
        if section in sections:
            _, length, start = sections[section]
            report[key] = sorted(set(x.decode(errors='replace') for x in raw[start:start + length].split(b'\0') if x))
    for name in archive.namelist():
        if not name.endswith('.nib'):
            continue
        objects = plistlib.loads(archive.read(name))['$objects']
        def value(v):
            if isinstance(v, plistlib.UID):
                obj = objects[v.data]
                if isinstance(obj, str):
                    return obj
                if isinstance(obj, dict):
                    return {'object': v.data, 'class': classname(obj)}
                return {'object': v.data}
            if isinstance(v, list):
                return [value(x) for x in v]
            if isinstance(v, dict):
                return {k: value(x) for k, x in v.items()}
            return v
        def classname(obj):
            ref = obj.get('$class')
            return objects[ref.data].get('$classname') if isinstance(ref, plistlib.UID) else obj.get('$classname')
        classes = collections.Counter()
        records = []
        keys = collections.Counter()
        for index, obj in enumerate(objects):
            if not isinstance(obj, dict) or '$class' not in obj:
                continue
            cls = classname(obj)
            classes[cls] += 1
            keys.update(obj.keys())
            if cls in ('UINavigationBar', 'UINavigationItem', 'UIBarButtonItem', 'UINavigationController', 'UIClassSwapper', 'UIRuntimeOutletConnection', 'UIRuntimeEventConnection'):
                records.append({'object': index, 'class': cls, 'fields': value(obj)})
        report['nibs'][name.rsplit('/', 1)[-1]] = {'classes': dict(classes), 'keys': dict(keys), 'navigation_and_connections': records}
runtime_classes = set()
for source in Path('touchHLE-src/src').rglob('*.rs'):
    runtime_classes.update(re.findall(r'@implementation\s+(\w+)', source.read_text(errors='replace')))
report['missing_import_classes'] = sorted(set(report.get('objc_import_classes', [])) - runtime_classes)
report['missing_nib_classes'] = sorted({cls for nib in report['nibs'].values() for cls in nib['classes']} - runtime_classes)
report['scope'] = 'Metadata inventory; dynamic selector execution and UI behavior are not verified.'
destination = Path('pokedex-ui-requirements.json')
destination.write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
print(json.dumps({'report': str(destination), 'sha256': report['sha256'], 'missing_import_classes': report['missing_import_classes'], 'missing_nib_classes': report['missing_nib_classes']}, indent=2))
