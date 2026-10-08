#!/bin/bash
set -euo pipefail
# Prepare a fresh source snapshot, dependencies and real Android assets first.
bash "$(dirname -- "${BASH_SOURCE[0]}")/build_a64_apk.sh"
export JAVA_HOME=/usr/lib/jvm/java-17-openjdk-amd64
export ANDROID_HOME=/home/ben/android-sdk
export ANDROID_NDK_HOME=/home/ben/android-sdk/ndk/25.2.9519653
export PATH=/home/ben/.cargo/bin:$JAVA_HOME/bin:$PATH
cd /home/ben/touchHLE-a64-integration/android
python3 - <<'PY'
from pathlib import Path
p=Path('app/build.gradle.kts')
p.write_text(p.read_text().replace('applicationId = "org.touchhle.android"', 'applicationId = "org.touchhle.android.a64test"'))
PY
/home/ben/gradle-8.11.1/bin/gradle --no-daemon assembleDebug
cp app/build/outputs/apk/debug/app-debug.apk '/mnt/c/Users/Ben/Downloads/Godot_v4.7.2-stable_mono_win64/iOS on android/PlayCover-A-9Pro-test.apk'
