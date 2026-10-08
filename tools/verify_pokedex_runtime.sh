#!/bin/bash
set -euo pipefail
source ~/.cargo/env
# Run after build_9pro_test.sh, against that same isolated source snapshot.
cd /home/ben/touchHLE-a64-integration
export CARGO_TARGET_DIR=/home/ben/touchHLE-a64/target
cargo test --offline --features a64 --lib
