#!/bin/bash
BIN=/mnt/c/Users/Ben/AppData/Local/Temp/claude/C--Users-Ben-Downloads-Godot-v4-7-2-stable-mono-win64/c3041004-6d17-45c5-8a53-8ba8147cfd23/scratchpad/flap
LLVM=~/android-sdk/ndk/25.2.9519653/toolchains/llvm/prebuilt/linux-x86_64/bin
$LLVM/llvm-objdump --macho --arch=armv7 --triple=thumbv7-apple-ios -d --no-show-raw-insn $BIN 2>/dev/null > /tmp/flap_dis.txt
wc -l /tmp/flap_dis.txt
grep -n -E "^ +a2(7[0-9a-f]|8[0-9a-f]):" /tmp/flap_dis.txt | head -3
awk '/^ +a2[0-9a-f][0-9a-f]:/ {print}' /tmp/flap_dis.txt | sed -n 1,60p
