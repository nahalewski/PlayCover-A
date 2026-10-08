#!/bin/bash
set -euo pipefail
source ~/.cargo/env
src='/mnt/c/Users/Ben/Downloads/Godot_v4.7.2-stable_mono_win64/iOS on android/touchHLE-src'
dst='/home/ben/touchHLE-a64-integration'
cp "$src/src/cpu.rs" "$dst/src/cpu.rs"
export CARGO_TARGET_DIR=/home/ben/touchHLE-a64/target
cd "$dst"
cargo test --offline --features a64 --lib diagnostic_tests
