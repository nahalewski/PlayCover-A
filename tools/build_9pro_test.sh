#!/bin/bash
set -euo pipefail
# Kept for old notes: there is now ONE app (32-bit and 64-bit runtimes, application id
# org.touchhle.android), so this just builds it with build_a64_apk.sh and copies the APK
# to the name the older evidence scripts expect.
bash "$(dirname -- "${BASH_SOURCE[0]}")/build_a64_apk.sh"
src='/mnt/c/Users/Ben/Downloads/Godot_v4.7.2-stable_mono_win64/iOS on android'
cp "$src/PlayCover-A-arm64-experimental.apk" "$src/PlayCover-A-9Pro-test.apk"
