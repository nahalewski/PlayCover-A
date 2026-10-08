#!/bin/bash
set -euo pipefail
source ~/.cargo/env
src='/mnt/c/Users/Ben/Downloads/Godot_v4.7.2-stable_mono_win64/iOS on android/touchHLE-src'
dst="$HOME/touchHLE-a64-integration"
mkdir -p "$dst"
rsync -a --exclude target --exclude .git --exclude vendor --exclude android/app/src/main/assets/touchHLE_dylibs --exclude android/app/src/main/assets/touchHLE_fonts "$src/" "$dst/"
ln -sfn "$HOME/touchHLE-a64/vendor" "$dst/vendor"
export CARGO_TARGET_DIR="$HOME/touchHLE-a64/target"
cd "$dst"
cargo test --offline -p touchHLE_dynarmic_wrapper --features a64
cargo test --offline --features a64 --lib a64
cargo run --offline --features a64 -- --a64-selftest
python3 "$src/../tools/test_a64_bundle.py" "$CARGO_TARGET_DIR/debug/touchHLE" "$dst/tests/a64/hello_arm64.macho"
python3 "$src/../tools/test_a64_linking.py" "$CARGO_TARGET_DIR/debug/touchHLE" "$dst/tests/a64"
