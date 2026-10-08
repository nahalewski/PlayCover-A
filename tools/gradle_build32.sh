#!/bin/bash
source ~/.cargo/env
export ANDROID_HOME=~/android-sdk
export ANDROID_SDK_ROOT=$ANDROID_HOME
export ANDROID_SDK_HOME=$ANDROID_HOME
export ANDROID_NDK_HOME=$ANDROID_HOME/ndk/25.2.9519653
export ANDROID_NDK=$ANDROID_NDK_HOME
export JAVA_HOME=/usr/lib/jvm/java-17-openjdk-amd64

# Gradle 8.11.1 (project has no wrapper)
if [ ! -x ~/gradle-8.11.1/bin/gradle ]; then
  wget -q https://services.gradle.org/distributions/gradle-8.11.1-bin.zip -O /tmp/gradle.zip
  unzip -q /tmp/gradle.zip -d ~
fi
export PATH=~/gradle-8.11.1/bin:$PATH

# SDK pieces the project asks for (compileSdk 31)
yes | $ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager --licenses >/dev/null 2>&1
$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager "platforms;android-31" "build-tools;34.0.0" >/dev/null 2>&1

# Refresh source copy (keep vendor/boost already unpacked in ~/touchHLE)
SRC="/mnt/c/Users/Ben/Downloads/Godot_v4.7.2-stable_mono_win64/iOS on android/touchHLE-src"
rsync -a --exclude target --exclude vendor/boost --exclude .git "$SRC/" ~/touchHLE/
# 32-bit build only: if the shared tree is part-way through registering the ARM64 ARC helper
# (a64_foundation_startup.rs already calls it but a64.rs has no mod line yet), register it in
# this private copy so the build is not blocked. The shared tree is not touched.
if ! grep -q "mod objc_arc_services;" ~/touchHLE/src/a64.rs && grep -q "super::objc_arc_services" ~/touchHLE/src/a64_foundation_startup.rs; then
  sed -i '/^mod mprotect;/a #[path = "a64_objc_arc_services.rs"]\nmod objc_arc_services;' ~/touchHLE/src/a64.rs
  echo "patched private copy: registered objc_arc_services"
fi

# Same for the CF number helpers (Codex adds the files before the mod lines).
# Any a64_<name>.rs that other a64 files already reference as `super::<name>` but that has no mod line yet.
for f in ~/touchHLE/src/a64_*.rs; do
  m=$(basename "$f" .rs); m=${m#a64_}
  case "$m" in *_tests) continue;; esac
  if grep -qlE "super::$m(::|;|,| |\\))" ~/touchHLE/src/a64*.rs 2>/dev/null && ! grep -q "mod $m;" ~/touchHLE/src/a64.rs; then
    printf '#[path = "a64_%s.rs"]
mod %s;
' "$m" "$m" >> ~/touchHLE/src/a64.rs
    echo "patched private copy: registered $m"
  fi
done

# Git symlinks were checked out as text files on Windows; resolve them for real here.
cd ~/touchHLE/android/app/src/main
for pair in \
  "assets/touchHLE_default_options.txt:../../../../../touchHLE_default_options.txt" \
  "assets/touchHLE_dylibs:../../../../../touchHLE_dylibs" \
  "assets/touchHLE_fonts:../../../../../touchHLE_fonts" \
  "res/drawable-nodpi/icon.png:../../../../../../res/icon.png" \
  "res/drawable-nodpi/icon_preview.png:../../../../../../res/icon_preview.png" \
  "res/drawable-nodpi/icon_unofficial.png:../../../../../../res/icon_unofficial.png"; do
  link="${pair%%:*}"; tgt="${pair#*:}"
  rm -rf "$link"
  cp -r "$(dirname "$link")/$tgt" "$link"
  echo "resolved $link ($(du -sh "$link" | cut -f1))"
done

cd ~/touchHLE/android
printf 'sdk.dir=%s\nndk.dir=%s\n' "$ANDROID_HOME" "$ANDROID_NDK_HOME" > local.properties
gradle --no-daemon assembleRelease > ~/gradle_build.log 2>&1
echo "GRADLE_EXIT=$?" >> ~/gradle_build.log
tail -40 ~/gradle_build.log
