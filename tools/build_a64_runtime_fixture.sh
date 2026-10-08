#!/bin/bash
set -euo pipefail
workspace=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
src="$workspace/touchHLE-src/tests/a64"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
clang-18 -target arm64-apple-ios14.0 -c "$src/runtime_services.s" -o "$work/runtime.o"
ld64.lld-18 -arch arm64 -platform_version ios 14.0 16.0 -fixup_chains \
    -e _main -o "$src/runtime_services.macho" "$work/runtime.o"
llvm-objdump-18 --disassemble "$src/runtime_services.macho"
clang-18 -target arm64-apple-ios14.0 -c "$src/runtime_initializer.s" -o "$work/initializer.o"
ld64.lld-18 -arch arm64 -platform_version ios 14.0 16.0 -fixup_chains \
    -e _main -o "$src/runtime_initializer.macho" "$work/initializer.o"
python3 - "$src" <<'PY'
import pathlib, plistlib, sys, zipfile
root = pathlib.Path(sys.argv[1])
prefix = 'Payload/RuntimeServices.app/'
info = {'CFBundleIdentifier': 'local.playcover.runtime_services',
        'CFBundleName': 'RuntimeServices', 'CFBundleExecutable': 'RuntimeServices',
        'CFBundleVersion': '1', 'MinimumOSVersion': '14.0'}
with zipfile.ZipFile(root / 'runtime_services.ipa', 'w', zipfile.ZIP_DEFLATED) as archive:
    archive.writestr(prefix + 'Info.plist', plistlib.dumps(info))
    archive.write(root / 'runtime_services.macho', prefix + 'RuntimeServices')
prefix = 'Payload/RuntimeInitializer.app/'
info = {'CFBundleIdentifier': 'local.playcover.runtime_initializer',
        'CFBundleName': 'RuntimeInitializer', 'CFBundleExecutable': 'RuntimeInitializer',
        'CFBundleVersion': '1', 'MinimumOSVersion': '14.0'}
with zipfile.ZipFile(root / 'runtime_initializer.ipa', 'w', zipfile.ZIP_DEFLATED) as archive:
    archive.writestr(prefix + 'Info.plist', plistlib.dumps(info))
    archive.write(root / 'runtime_initializer.macho', prefix + 'RuntimeInitializer')
print('Built runtime services and initializer Mach-O/IPA fixtures; expected exit status 42')
PY
