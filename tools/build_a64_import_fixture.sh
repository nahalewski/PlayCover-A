#!/bin/bash
set -euo pipefail
src='/mnt/c/Users/Ben/Downloads/Godot_v4.7.2-stable_mono_win64/iOS on android/touchHLE-src/tests/a64'
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
clang-18 -target arm64-apple-ios14.0 -c "$src/import_provider.s" -o "$work/provider.o"
clang-18 -target arm64-apple-ios14.0 -c "$src/import_client.s" -o "$work/client.o"
ld64.lld-18 -arch arm64 -dylib -platform_version ios 14.0 16.0 -fixup_chains -install_name '@rpath/libAnswer.dylib' -o "$src/libAnswer.dylib" "$work/provider.o"
ld64.lld-18 -arch arm64 -platform_version ios 14.0 16.0 -fixup_chains -e _main -rpath '@executable_path/Frameworks' -o "$src/import_client.macho" "$work/client.o" "$src/libAnswer.dylib"
clang-18 -target arm64-apple-ios14.0 -c "$src/initializer_provider.s" -o "$work/initializer.o"
ld64.lld-18 -arch arm64 -dylib -platform_version ios 14.0 16.0 -fixup_chains -install_name '@rpath/libInitialized.dylib' -o "$src/libInitialized.dylib" "$work/initializer.o"
ld64.lld-18 -arch arm64 -platform_version ios 14.0 16.0 -fixup_chains -e _main -rpath '@executable_path/Frameworks' -o "$src/initializer_client.macho" "$work/client.o" "$src/libInitialized.dylib"
