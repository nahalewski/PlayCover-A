#!/bin/bash
set -euo pipefail
workspace=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
src="$workspace/touchHLE-src/tests/a64"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
clang-18 -target arm64-apple-ios12.0 -c "$src/legacy_provider.s" -o "$work/provider.o"
clang-18 -target arm64-apple-ios12.0 -c "$src/import_client.s" -o "$work/eager.o"
clang-18 -target arm64-apple-ios12.0 -c "$src/legacy_lazy_client.s" -o "$work/lazy.o"
ld64.lld-18 -arch arm64 -dylib -platform_version ios 12.0 16.0 -no_fixup_chains \
    -install_name '@rpath/libLegacyAnswer.dylib' -o "$src/libLegacyAnswer.dylib" "$work/provider.o"
ld64.lld-18 -arch arm64 -platform_version ios 12.0 16.0 -no_fixup_chains \
    -e _main -rpath '@executable_path/Frameworks' -o "$src/legacy_client.macho" \
    "$work/eager.o" "$src/libLegacyAnswer.dylib"
ld64.lld-18 -arch arm64 -platform_version ios 12.0 16.0 -no_fixup_chains \
    -e _main -rpath '@executable_path/Frameworks' -o "$src/legacy_lazy_client.macho" \
    "$work/lazy.o" "$src/libLegacyAnswer.dylib"
python3 - "$src" <<'PY'
import pathlib, plistlib, struct, sys, zipfile
root = pathlib.Path(sys.argv[1])

def streams(path):
    data = path.read_bytes()
    offset = 32
    result = None
    for _ in range(struct.unpack_from('<I', data, 16)[0]):
        command, size = struct.unpack_from('<II', data, offset)
        if command == 0x80000034:
            raise ValueError(f'{path.name} unexpectedly uses chained fixups')
        if command in (0x22, 0x80000022):
            result = dict(zip(('rebase', 'bind', 'weak_bind', 'lazy_bind', 'exports'),
                              struct.unpack_from('<10I', data, offset + 8)[1::2]))
        offset += size
    if result is None:
        raise ValueError(f'{path.name} has no LC_DYLD_INFO')
    return result

provider = root / 'libLegacyAnswer.dylib'
assert streams(provider)['rebase'] > 0
assert streams(root / 'legacy_client.macho')['bind'] > 0
assert streams(root / 'legacy_lazy_client.macho')['lazy_bind'] > 0
for stem in ('legacy_client', 'legacy_lazy_client'):
    name = 'LegacyClient' if stem == 'legacy_client' else 'LegacyLazyClient'
    prefix = f'Payload/{name}.app/'
    info = {'CFBundleIdentifier': f'local.playcover.{stem}', 'CFBundleName': name,
            'CFBundleExecutable': name, 'CFBundleVersion': '1', 'MinimumOSVersion': '12.0'}
    with zipfile.ZipFile(root / (stem + '.ipa'), 'w', zipfile.ZIP_DEFLATED) as archive:
        archive.writestr(prefix + 'Info.plist', plistlib.dumps(info))
        archive.write(root / (stem + '.macho'), prefix + name)
        archive.write(provider, prefix + 'Frameworks/' + provider.name)
    print(stem, streams(root / (stem + '.macho')))
print(provider.name, streams(provider))
PY
