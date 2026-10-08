"""Read-only signed legacy bind operand audit for the actual Quest image."""
import pathlib,json,zipfile
root=pathlib.Path(__file__).resolve().parents[1]
src=(root/'tools/inspect_legacy_ipa_runtime.py').read_text().split('ipa, cache_root, output =')[0]
src=src.replace("ordinal = -3 if kind == 'weak' else 0", "addend = 0\n    ordinal = -3 if kind == 'weak' else 0")
src=src.replace("elif opcode in (0x60, 0x70, 0x80):", "elif opcode == 0x60:\n            raw, offset = uleb(data, offset)\n            count = 1\n            value = raw\n            while raw >= 128:\n                raw >>= 7\n                count += 1\n            addend = value - (1 << (count * 7)) if data[offset-1] & 64 else value\n        elif opcode in (0x70, 0x80):")
src=src.replace("references.add((ordinal, name, bool(flags & 1), typ))", "references.add((ordinal, name, bool(flags & 1), typ))\n            if addend: print(kind, name, 'ordinal', ordinal, 'weak', bool(flags & 1), 'addend', addend)")
ns={};exec(compile(src,'legacy-audit','exec'),ns)
report=json.loads((root/'ios-runtime/legacy-cpp-consumer-audit.json').read_text())[-1]
with zipfile.ZipFile(root/'PokemonQuest-device.ipa') as z:
 image=next(x for x in report['images'] if x['path'].endswith('/pokemonquest'));data,_=ns['thin_arm64'](z.read(image['path']))
 for kind in ('bind','lazy','weak'):
  stream=image['streams'].get(kind)
  if stream:ns['stream'](data[stream['offset']:stream['offset']+stream['size']],kind,image['dependencies'])
