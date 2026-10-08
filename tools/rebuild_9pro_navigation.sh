#!/bin/bash
set -euo pipefail
# Finish an incremental navigation change after build_9pro_test.sh.
source ~/.cargo/env
src='/mnt/c/Users/Ben/Downloads/Godot_v4.7.2-stable_mono_win64/iOS on android/touchHLE-src'
dst='/home/ben/touchHLE-a64-integration'
python3 - "$dst" <<'PY'
from pathlib import Path
import sys
assert 'applicationId = "org.touchhle.android.a64test"' in (Path(sys.argv[1])/'android/app/build.gradle.kts').read_text()
PY
cp "$src/src/frameworks/uikit/ui_view/ui_navigation_bar.rs" "$dst/src/frameworks/uikit/ui_view/ui_navigation_bar.rs"
export JAVA_HOME=/usr/lib/jvm/java-17-openjdk-amd64
export ANDROID_HOME=/home/ben/android-sdk
export ANDROID_NDK_HOME=/home/ben/android-sdk/ndk/25.2.9519653
cd "$dst/android"
/home/ben/gradle-8.11.1/bin/gradle --no-daemon assembleDebug
cp app/build/outputs/apk/debug/app-debug.apk "$src/../PlayCover-A-9Pro-test.apk"
