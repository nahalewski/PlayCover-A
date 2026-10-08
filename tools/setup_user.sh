#!/bin/bash
set -e
cd ~
# Rust
if [ ! -x ~/.cargo/bin/rustup ]; then
  curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
fi
source ~/.cargo/env
rustup target add aarch64-linux-android
cargo install cargo-ndk

# Android SDK + NDK
export ANDROID_HOME=~/android-sdk
mkdir -p $ANDROID_HOME/cmdline-tools
if [ ! -d $ANDROID_HOME/cmdline-tools/latest ]; then
  wget -q https://dl.google.com/android/repository/commandlinetools-linux-11076708_latest.zip -O /tmp/clt.zip
  unzip -q /tmp/clt.zip -d /tmp/clt
  mv /tmp/clt/cmdline-tools $ANDROID_HOME/cmdline-tools/latest
fi
yes | $ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager --licenses >/dev/null || true
$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager "platform-tools" "platforms;android-34" "build-tools;34.0.0" "ndk;25.2.9519653"
ls $ANDROID_HOME/ndk

# Source copy on the native WSL filesystem (much faster than /mnt/c)
SRC="/mnt/c/Users/Ben/Downloads/Godot_v4.7.2-stable_mono_win64/iOS on android/touchHLE-src"
mkdir -p ~/touchHLE
rsync -a --exclude target "$SRC/" ~/touchHLE/

# Boost headers
if [ ! -d ~/touchHLE/vendor/boost/boost ]; then
  wget -q https://archives.boost.io/release/1.81.0/source/boost_1_81_0.tar.gz -O /tmp/boost.tgz
  mkdir -p ~/touchHLE/vendor/boost
  tar -xzf /tmp/boost.tgz -C ~/touchHLE/vendor/boost --strip-components=1
fi
ls ~/touchHLE/vendor/boost | head -3
echo SETUP_DONE
