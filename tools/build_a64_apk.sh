#!/bin/bash
set -euo pipefail
source ~/.cargo/env
# Same debug keystore as the 32-bit/release builds (ANDROID_SDK_HOME/.android/debug.keystore), so updates install over each other.
export ANDROID_SDK_HOME="$HOME/android-sdk"
export ANDROID_HOME="$HOME/android-sdk"
export ANDROID_SDK_ROOT="$ANDROID_HOME"
export ANDROID_NDK_HOME="$ANDROID_HOME/ndk/25.2.9519653"
export ANDROID_NDK="$ANDROID_NDK_HOME"
export JAVA_HOME=/usr/lib/jvm/java-17-openjdk-amd64
src='/mnt/c/Users/Ben/Downloads/Godot_v4.7.2-stable_mono_win64/iOS on android/touchHLE-src'
dst="$HOME/touchHLE-a64-integration"
rsync -a --exclude target --exclude .git --exclude vendor --exclude android/.gradle --exclude android/app/build --exclude android/app/src/main/assets/touchHLE_dylibs --exclude android/app/src/main/assets/touchHLE_fonts "$src/" "$dst/"
ln -sfn "$HOME/touchHLE-a64/vendor" "$dst/vendor"
python3 - "$dst" <<'PY'
import pathlib, shutil, sys
root = pathlib.Path(sys.argv[1])
for target, source in {
    'android/app/src/main/assets/touchHLE_default_options.txt': 'touchHLE_default_options.txt',
    'android/app/src/main/assets/touchHLE_dylibs': 'touchHLE_dylibs',
    'android/app/src/main/assets/touchHLE_fonts': 'touchHLE_fonts',
    'android/app/src/main/res/drawable-nodpi/icon.png': 'res/icon.png',
    'android/app/src/main/res/drawable-nodpi/icon_preview.png': 'res/icon_preview.png',
    'android/app/src/main/res/drawable-nodpi/icon_unofficial.png': 'res/icon_unofficial.png',
}.items():
    dest = root / target
    if dest.is_symlink() or dest.is_file(): dest.unlink()
    elif dest.is_dir(): shutil.rmtree(dest)
    dest.parent.mkdir(parents=True, exist_ok=True)
    origin = root / source
    if origin.is_dir(): shutil.copytree(origin, dest)
    else: shutil.copy2(origin, dest)
# Reuse this task's native build cache, separate from Claude's build directory.
target = root / 'target'
cache = pathlib.Path.home() / 'touchHLE-a64/target'
if not target.is_symlink():
    if target.exists():
        backup = root / 'target-before-apk'
        if backup.exists(): raise RuntimeError('Target backup already exists')
        target.rename(backup)
    target.symlink_to(cache, target_is_directory=True)
PY
cd "$dst/android"
printf 'sdk.dir=%s\nndk.dir=%s\n' "$ANDROID_HOME" "$ANDROID_NDK_HOME" > local.properties
"$HOME/gradle-8.11.1/bin/gradle" --no-daemon ${NDK_VERSION:+-PndkVersion=$NDK_VERSION} assembleDebug
cp app/build/outputs/apk/debug/app-debug.apk "$src/../PlayCover-A-arm64-experimental.apk"
