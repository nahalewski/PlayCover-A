#!/bin/bash
set -euo pipefail
source ~/.cargo/env
src='/mnt/c/Users/Ben/Downloads/Godot_v4.7.2-stable_mono_win64/iOS on android/touchHLE-src'
dst='/home/ben/touchHLE-a64-integration'
cp "$src/src/cpu/dynarmic_wrapper/a64.cpp" "$dst/src/cpu/dynarmic_wrapper/a64.cpp"
cp "$src/src/cpu/dynarmic_wrapper/a64.rs" "$dst/src/cpu/dynarmic_wrapper/a64.rs"
cp "$src/src/a64.rs" "$dst/src/a64.rs"
cd "$dst"
export CARGO_TARGET_DIR=/home/ben/touchHLE-a64/target
cargo test -p touchHLE_dynarmic_wrapper --features a64 simd_accessors -- --nocapture
cargo test -p touchHLE_dynarmic_wrapper --features a64 objc_nil_movi -- --nocapture
cargo test --lib a64_large_initializer_table -- --nocapture
