#!/bin/bash
set -euo pipefail
source ~/.cargo/env
src='/mnt/c/Users/Ben/Downloads/Godot_v4.7.2-stable_mono_win64/iOS on android/touchHLE-src'
dst='/home/ben/touchHLE-a64-integration'
python3 - "$dst" <<'PY'
from pathlib import Path
import sys
assert 'applicationId = "org.touchhle.android.a64test"' in (Path(sys.argv[1])/'android/app/build.gradle.kts').read_text()
PY
cp "$src/android/app/src/main/java/org/touchhle/android/LauncherActivity.kt" "$dst/android/app/src/main/java/org/touchhle/android/LauncherActivity.kt"
cp "$src/android/app/src/main/java/org/touchhle/android/AppleStoreActivity.kt" "$dst/android/app/src/main/java/org/touchhle/android/AppleStoreActivity.kt"
cp "$src/android/app/src/main/java/org/touchhle/android/AppleStoreCatalog.kt" "$dst/android/app/src/main/java/org/touchhle/android/AppleStoreCatalog.kt"
mkdir -p "$dst/android/app/src/main/res/xml"
cp "$src/android/app/src/main/res/xml/apple_companion_network.xml" "$dst/android/app/src/main/res/xml/apple_companion_network.xml"
cp "$src/android/app/src/main/java/org/touchhle/android/RepoSources.kt" "$dst/android/app/src/main/java/org/touchhle/android/RepoSources.kt"
cp "$src/android/app/src/main/java/org/touchhle/android/IpaDownloads.kt" "$dst/android/app/src/main/java/org/touchhle/android/IpaDownloads.kt"
cp "$src/android/app/src/main/java/org/touchhle/android/IpaDownloadService.kt" "$dst/android/app/src/main/java/org/touchhle/android/IpaDownloadService.kt"
cp "$src/android/app/src/main/AndroidManifest.xml" "$dst/android/app/src/main/AndroidManifest.xml"
cp "$src/android/app/src/main/java/org/touchhle/android/HomeShortcuts.kt" "$dst/android/app/src/main/java/org/touchhle/android/HomeShortcuts.kt"
cp "$src/android/app/src/main/java/org/touchhle/android/IpaInfo.kt" "$dst/android/app/src/main/java/org/touchhle/android/IpaInfo.kt"
cp "$src/android/app/src/main/java/org/touchhle/android/SymbolIndex.kt" "$dst/android/app/src/main/java/org/touchhle/android/SymbolIndex.kt"
cp "$src/android/app/src/main/java/org/touchhle/android/Compat.kt" "$dst/android/app/src/main/java/org/touchhle/android/Compat.kt"
# The compatibility UI calls this native symbol index; stage its real export
# and the registered Security provider together with the Kotlin callers.
cp "$src/src/dyld/dylib_list.rs" "$dst/src/dyld/dylib_list.rs"
cp "$src/src/frameworks.rs" "$dst/src/frameworks.rs"
cp "$src/src/frameworks/security.rs" "$dst/src/frameworks/security.rs"
cp "$src/android/app/src/main/java/org/touchhle/android/MainActivity.java" "$dst/android/app/src/main/java/org/touchhle/android/MainActivity.java"
cp "$src/android/app/src/main/java/org/touchhle/android/CrashLog.java" "$dst/android/app/src/main/java/org/touchhle/android/CrashLog.java"
cp "$src/android/app/src/main/java/org/touchhle/android/EmulatorSettings.kt" "$dst/android/app/src/main/java/org/touchhle/android/EmulatorSettings.kt"
mkdir -p "$dst/android/app/src/main/assets"
cp "$src/android/app/src/main/assets/games.json" "$dst/android/app/src/main/assets/games.json"
cp "$src/android/app/src/test/java/org/touchhle/android/RepoSourcesTest.kt" "$dst/android/app/src/test/java/org/touchhle/android/RepoSourcesTest.kt"
cp "$src/android/app/src/test/java/org/touchhle/android/IpaDownloadsTest.kt" "$dst/android/app/src/test/java/org/touchhle/android/IpaDownloadsTest.kt"
cp "$src/android/app/src/test/java/org/touchhle/android/HomeShortcutsTest.kt" "$dst/android/app/src/test/java/org/touchhle/android/HomeShortcutsTest.kt"
cp "$src/android/app/src/test/java/org/touchhle/android/IpaInfoTest.kt" "$dst/android/app/src/test/java/org/touchhle/android/IpaInfoTest.kt"
cp "$src/android/app/src/test/java/org/touchhle/android/IpaInfoMetadataTest.kt" "$dst/android/app/src/test/java/org/touchhle/android/IpaInfoMetadataTest.kt"
cp "$src/android/app/src/test/java/org/touchhle/android/CrashLogTest.java" "$dst/android/app/src/test/java/org/touchhle/android/CrashLogTest.java"
cp "$src/android/app/src/test/java/org/touchhle/android/EmulatorSettingsTest.kt" "$dst/android/app/src/test/java/org/touchhle/android/EmulatorSettingsTest.kt"
cp -r "$src/android/app/src/main/res/drawable" "$dst/android/app/src/main/res/"
cp -r "$src/android/app/src/main/res/mipmap-anydpi" "$dst/android/app/src/main/res/"
cp -r "$src/android/app/src/main/res/mipmap-anydpi-v26" "$dst/android/app/src/main/res/"
cp "$src/android/app/src/main/res/drawable-nodpi/playcover_a_foreground.png" "$dst/android/app/src/main/res/drawable-nodpi/"
python3 - "$dst" <<'PY'
from pathlib import Path
import sys
p = Path(sys.argv[1]) / 'android/app/build.gradle.kts'
s = p.read_text().replace('manifestPlaceholders["icon"] = join("@drawable/icon", "_", branding.lowercase())', 'manifestPlaceholders["icon"] = "@mipmap/ic_launcher"')
p.write_text(s)
PY
export JAVA_HOME=/usr/lib/jvm/java-17-openjdk-amd64
export ANDROID_HOME=/home/ben/android-sdk
export ANDROID_NDK_HOME=/home/ben/android-sdk/ndk/25.2.9519653
cd "$dst/android"
if [[ "${PLAYCOVER_UI_ONLY:-0}" == 1 ]]; then
    /home/ben/gradle-8.11.1/bin/gradle --no-daemon :app:testDebugUnitTest assembleDebug -x :app:buildCargoNdkDebug
else
    /home/ben/gradle-8.11.1/bin/gradle --no-daemon :app:testDebugUnitTest assembleDebug
fi
cp app/build/outputs/apk/debug/app-debug.apk "$src/../PlayCover-A-9Pro-test.apk"
