#!/bin/bash
BIN=/mnt/c/Users/Ben/AppData/Local/Temp/claude/C--Users-Ben-Downloads-Godot-v4-7-2-stable-mono-win64/c3041004-6d17-45c5-8a53-8ba8147cfd23/scratchpad/flap
LLVM=~/android-sdk/ndk/25.2.9519653/toolchains/llvm/prebuilt/linux-x86_64/bin
[ -f /tmp/flap_dis_arm.txt ] || $LLVM/llvm-objdump --macho --arch=armv7 --triple=armv7-apple-ios -d --no-show-raw-insn $BIN 2>/dev/null > /tmp/flap_dis_arm.txt
lo=$1; hi=$2
awk -v lo="$lo" -v hi="$hi" '{ a=$1; sub(":","",a); v=strtonum("0x" a); if (v>=strtonum("0x" lo) && v<=strtonum("0x" hi)) print }' /tmp/flap_dis_arm.txt
