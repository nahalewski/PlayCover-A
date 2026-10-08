#!/usr/bin/env bash
# Coordinator only: freezes the current tree and owns the native build.
set -euo pipefail
workspace="$(cd -- "$(dirname -- "$0")/.." && pwd)"
native=/home/ben/touchHLE-a64-integration
target=/home/ben/touchHLE-a64/target
out="$workspace/pixel-fold-tests/arm64-regression"
mkdir -p "$out"
exec 9>"$out/runner.lock"
flock -n 9 || { echo 'Another ARM64 regression runner is active' >&2; exit 1; }
python3 "$workspace/tools/ipa_inventory.py" --ipa-dir /mnt/c/Users/Ben/Desktop/ipa \
    --downloads /mnt/c/Users/Ben/Downloads --checkpoint --output "$out/inventory.json"
bash "$workspace/tools/build_bitmap_audio.sh" --snapshot-only
cd "$native"
CARGO_TARGET_DIR="$target" /home/ben/.cargo/bin/cargo test --lib --features a64 \
    --no-run --message-format=json > "$out/build.json" 2> "$out/build.log"
python3 "$workspace/tools/regress_arm64.py" --inventory "$out/inventory.json" \
    --build-json "$out/build.json" --native "$native" \
    --cache "$workspace/ios-runtime/cache/System/Library/Caches/com.apple.dyld/dyld_shared_cache_arm64" \
    --output "$out" "$@"
