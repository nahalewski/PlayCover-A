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
SRC="/mnt/c/Users/Ben/Downloads/Godot_v4.7.2-stable_mono_win64/iOS on android/touchHLE-src-agent"
mkdir -p ~/touchHLE-agent
rsync -a --exclude target --exclude vendor/boost --exclude .git "$SRC/" ~/touchHLE-agent/
mkdir -p ~/touchHLE-agent/vendor/boost
[ -d ~/touchHLE-agent/vendor/boost/boost ] || rsync -a ~/touchHLE/vendor/boost/ ~/touchHLE-agent/vendor/boost/

# Git symlinks were checked out as text files on Windows; resolve them for real here.
cd ~/touchHLE-agent/android/app/src/main
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

cd ~/touchHLE-agent/android
printf 'sdk.dir=%s\nndk.dir=%s\n' "$ANDROID_HOME" "$ANDROID_NDK_HOME" > local.properties
gradle --no-daemon assembleRelease > ~/gradle_build_agent.log 2>&1
echo "GRADLE_EXIT=$?" >> ~/gradle_build_agent.log
tail -40 ~/gradle_build_agent.log
