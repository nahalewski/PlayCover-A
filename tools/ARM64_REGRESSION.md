# ARM64 regression runner

The coordinator runs this in WSL, with no other native builder modifying the frozen integration checkout:

```bash
bash tools/run_arm64_regression.sh --timeout 120
```

It inventories `C:/Users/Ben/Desktop/ipa` recursively, and the Downloads and project root directories nonrecursively. It reads actual main executable Mach-O architecture/encryption metadata, hashes backups, and tests each unique unencrypted ARM64 IPA. Non-games are included rather than guessing categories. Duplicate, malformed, encrypted and non-ARM64 backups remain visible as separate inventory statuses.

The script freezes source once, builds the opt-in native harness once, and runs one independent bounded process per app. Workers can continue editing Windows source while the frozen WSL tree stays unchanged. Source hashes and the built test executable hash are recorded. No Android APK is built or device data changed.

The generic bundle reader resolves real embedded libraries and original shared-cache dependencies without forcing Terraria's paths onto other apps. Every eligible bundle runs the original libSystem initializer diagnostic if its dependencies can be resolved. UnityFramework is explicitly included only when present in the actual IPA. This is a first-boundary compatibility sweep, **not a title-screen/gameplay test**. Successful diagnostic collection is never reported as a boot. Later failures remain unknown until the first gap is fixed and the batch rerun.

Results go to `pixel-fold-tests/arm64-regression/REPORT.md`, `report.json`, and per-app `.log` files. The report groups first failures with addresses and bundle paths normalized, preserving unresolved API names. `inventory.json` is checkpointed during scanning.

For an existing frozen build/inventory, run `regress_arm64.py --help` and pass `--inventory`, `--build-json`, `--cache`, and `--output` to avoid unnecessary recompilation/scanning. The build JSON must be Cargo's successful `--message-format=json` output for `cargo test --lib --features a64 --no-run`.

`summarize_arm64_regression.py --inventory INVENTORY.json --baseline REPORT.json --retest RETEST.json --output ARM64_REGRESSION.md` merges observations by backup SHA256 and annotates actual iPhone/tvOS platform metadata. Repeat `--retest` in chronological order for later patch rounds. The latest observation wins; original raw boundaries and log references are retained in the adjacent JSON. Known development fixtures stay distinct from game backups.
