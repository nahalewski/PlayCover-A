#!/bin/bash
# Usage: missing_symbols.sh <path to extracted armv7 binary> [objc]
# Lists undefined C symbols (functions/constants) the binary imports that touchHLE's source doesn't export.
# With "objc" as 2nd arg, lists imported Objective-C classes touchHLE doesn't implement.
BIN="$1"
LLVM=~/android-sdk/ndk/25.2.9519653/toolchains/llvm/prebuilt/linux-x86_64/bin
SRC="/mnt/c/Users/Ben/Downloads/Godot_v4.7.2-stable_mono_win64/iOS on android/touchHLE-src/src"
T=$(mktemp -d)
if [ "$2" = "objc" ]; then
  $LLVM/llvm-nm --arch=armv7 -u "$BIN" 2>/dev/null | grep '_OBJC_CLASS_\$_' | sed 's/.*_OBJC_CLASS_\$_//' | sort -u > $T/imp.txt
  grep -rhoE "@implementation +[A-Za-z0-9_]+" "$SRC" | awk '{print $2}' | sort -u > $T/have.txt
  comm -23 $T/imp.txt $T/have.txt
  exit 0
fi
$LLVM/llvm-nm --arch=armv7 -u "$BIN" 2>/dev/null | awk '{print $NF}' | grep '^_' | grep -v 'OBJC_\|_objc_\|^__' | sed 's/^_//' | sort -u > $T/imp.txt
# everything touchHLE mentions as an identifier or quoted symbol anywhere in src
grep -rhoE "[A-Za-z_][A-Za-z0-9_]*" "$SRC" | sort -u > $T/have.txt
comm -23 $T/imp.txt $T/have.txt
