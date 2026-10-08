#!/bin/bash
BIN=/mnt/c/Users/Ben/AppData/Local/Temp/claude/C--Users-Ben-Downloads-Godot-v4-7-2-stable-mono-win64/c3041004-6d17-45c5-8a53-8ba8147cfd23/scratchpad/flap
LLVM=~/android-sdk/ndk/25.2.9519653/toolchains/llvm/prebuilt/linux-x86_64/bin
$LLVM/llvm-nm --arch=armv7 -u $BIN 2>/dev/null | grep "_OBJC_CLASS_\$_" | sed 's/.*_OBJC_CLASS_\$_//' | sort -u > /tmp/imported_classes.txt
wc -l /tmp/imported_classes.txt
SRC="/mnt/c/Users/Ben/Downloads/Godot_v4.7.2-stable_mono_win64/iOS on android/touchHLE-src/src"
grep -rhoE "@implementation +[A-Za-z0-9_]+" "$SRC" | awk '{print $2}' | sort -u > /tmp/have_classes.txt
wc -l /tmp/have_classes.txt
echo "== imported but not implemented by touchHLE:"
comm -23 /tmp/imported_classes.txt /tmp/have_classes.txt | tr '\n' ' '
echo
