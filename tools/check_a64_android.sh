#!/bin/bash
set -euo pipefail
source ~/.cargo/env
export ANDROID_HOME="$HOME/android-sdk"
export ANDROID_NDK_HOME="$ANDROID_HOME/ndk/25.2.9519653"
export ANDROID_NDK="$ANDROID_NDK_HOME"
export CARGO_TARGET_DIR="$HOME/touchHLE-a64/target"
cd "$HOME/touchHLE-a64-integration"
# The project's build.rs copies libc++ into this conventional path even when
# CARGO_TARGET_DIR points elsewhere.
mkdir -p target/aarch64-linux-android/debug
cargo ndk -t arm64-v8a -P 21 build --offline --lib --no-default-features --features touchHLE_openal_soft_wrapper/static,sdl2/bundled,a64
