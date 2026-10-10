# ARM64 device test loop (Samsung Galaxy Tab S11)

How to build, stage, launch and read results for a 64-bit iOS app (example:
Terraria 4.5.0) on the debuggable test package `org.touchhle.android`.
Last verified 2026-10-10 on SM-X930 (`gts11uwifi`), APK SHA256
`578217537a63381359593e94ca1b05405ceb3e7975da94b95ef6bd6d0e9944a1`.

Paths below are relative to the workspace parent folder (`iOS on android/`,
the folder that holds `ios-runtime/`, `pixel-fold-tests/` and `touchHLE-src/`).
Run the Python tools from that folder: they write evidence to
`pixel-fold-tests/` relative to the current directory.

## 0. Rules

- There is ONE app now (32-bit and 64-bit runtimes, `org.touchhle.android`). Never uninstall it or clear its data on a user device; staging only adds files under its private `files/ios-runtime` and `files/touchHLE_device_tests`. Do not touch other packages.
- One builder and one device launcher at a time.
- A diagnostic run that reaches an error is not a boot. Never report one as a
  title screen or gameplay.

## 1. Build (WSL, about 1 minute with cached natives)

```bash
wsl bash -lc "cd '/mnt/c/Users/Ben/Downloads/Godot_v4.7.2-stable_mono_win64/iOS on android/touchHLE-src' && bash tools/build_9pro_test.sh"
```

`build_a64_apk.sh` rsyncs the Windows tree into `~/touchHLE-a64-integration`
(uncommitted edits included), then `build_9pro_test.sh` rewrites the
applicationId to `org.touchhle.android` and builds a debug APK.
The output is `PlayCover-A-9Pro-test.apk` in the workspace parent. The 32-bit builder uses
`~/touchHLE` and does not collide with this one.

## 2. Install

```bash
adb devices -l     # the tablet is on wireless adb: adb-R52Y8066STA-QDNQZy._adb-tls-connect._tcp
adb -s SERIAL install -r PlayCover-A-9Pro-test.apk      # -r keeps app data
adb -s SERIAL shell sha256sum $(adb -s SERIAL shell pm path org.touchhle.android | sed s/package://)
```

The package must stay debuggable, because staging uses `run-as`.

## 3. Stage the cache and the IPA (once per fresh app data)

Terraria (minimum iOS 13, Unity, PlayFabParty, Metal) binds against the
**iOS 16.7.16 (20H392) cache** at
`ios-runtime/cache/System/Library/Caches/com.apple.dyld/`. Pokemon Quest and
Infinity Blade use the iOS 11.4.1 cache at `ios-runtime/legacy-cache/` instead
(see `tools/stage_legacy_tablet.py`). The manifest of the 44 iOS 16 files
needed (main file plus subcaches plus `.symbols`, but not `.a2s`) and their
SHA256 hashes is `pixel-fold-tests/tablet-ios16-runtime.json`.

```bash
C:/Python314/python.exe touchHLE-src/tools/stage_a64_device.py \
  --serial SERIAL --device gts11uwifi \
  --cache-manifest pixel-fold-tests/tablet-ios16-runtime.json \
  --cache-source ios-runtime/cache/System/Library/Caches/com.apple.dyld \
  --cache-dest ios-runtime/cache \
  --ipa C:/Users/Ben/Desktop/ipa/Terraria_4.5.0.ipa \
  --report pixel-fold-tests/terraria-tablet-stage-2026-10-10.json
```

This puts the files in app-private storage:

- `/data/user/0/org.touchhle.android/files/ios-runtime/cache/dyld_shared_cache_arm64*`
  (the subcaches must sit next to the main file)
- `/data/user/0/org.touchhle.android/files/touchHLE_device_tests/Terraria_4.5.0.ipa`

Each file is hashed locally and checked against the manifest. It is then pushed
to `/data/local/tmp`, verified, piped into `run-as ... cat`, and verified again
before an atomic `mv`. If an identical file already exists it is skipped. If a
different file exists, the script stops and does not overwrite it. A rerun resumes
per file: files that are already done are skipped, and a push that dropped
partway is sent again in full. Over wireless adb the full 3.3 GB plus the IPA
took 404 s (about 13 to 16 MB/s).

## 4. Launch and collect

Before each launch, wake the screen and check what has focus:

```bash
adb -s SERIAL shell 'input keyevent KEYCODE_WAKEUP; wm dismiss-keyguard; cmd statusbar collapse; dumpsys window | grep mCurrentFocus'
```

**Plain launch** (the normal user path, with no diagnostic flags):

```bash
C:/Python314/python.exe touchHLE-src/tools/test_ipa_device.py --ipa C:/Users/Ben/Desktop/ipa/Terraria_4.5.0.ipa \
  --serial SERIAL --adb-port 5037 --device gts11uwifi --existing-private-only \
  --label terraria-YYYY-MM-DD-plain --wait-seconds 30
```

**Cache-session diagnostic** (runs the original cached libSystem initializer
inside a retained process session; this is how the current frontier is
measured):

```bash
C:/Python314/python.exe touchHLE-src/tools/test_ipa_device.py --ipa C:/Users/Ben/Desktop/ipa/Terraria_4.5.0.ipa \
  --serial SERIAL --adb-port 5037 --device gts11uwifi --existing-private-only \
  --label terraria-YYYY-MM-DD-session --wait-seconds 45 \
  --extra-args "--no-error-popup --a64-cache-session-test=/data/user/0/org.touchhle.android/files/ios-runtime/cache/dyld_shared_cache_arm64"
```

Other probes go in the same `--extra-args` slot (see the option parser in `src/lib.rs`):
`--a64-cache-prepare=`, `--a64-cache-session-image-info-test=` (the legacy
iOS 11 path), `--a64-cache-entry-prefix-test=`, `--a64-cache-services-test=`.
Each one takes the path of the main cache file.

Each run writes `pixel-fold-tests/<label>.json` and `<label>.log` (plus
`<label>.png` if the test app still has focus). The fields to read are:

- `run_pid` / `outcome_scope_error`: the run is valid only if exactly one
  `:game` process was found after the unique launch marker.
- `runtime_error`: the first `touchHLE errored:` line from that PID. The
  `pc`, `lr`, `sp` and `x0..x5` values are inside it.
- `first_panic`: a Rust panic, if any.
- The `[a64] ...` lines in `.log` just before the error are the last host
  services and traps that ran.

When finished, run `adb -s SERIAL shell am force-stop org.touchhle.android`.

## 5. Desktop native harness (same code, no device, about 1 minute)

This runs the single test that `tools/run_arm64_regression.sh` would run for
one IPA. Use it instead of the full sweep, which inventories every IPA and
takes a lock:

```bash
# inside WSL, after build_9pro_test.sh has synced ~/touchHLE-a64-integration
cd ~/touchHLE-a64-integration && source ~/.cargo/env
ws='/mnt/c/Users/Ben/Downloads/Godot_v4.7.2-stable_mono_win64/iOS on android'
PLAYCOVER_NATIVE_IPA=/mnt/c/Users/Ben/Desktop/ipa/Terraria_4.5.0.ipa \
PLAYCOVER_NATIVE_CACHE="$ws/ios-runtime/cache/System/Library/Caches/com.apple.dyld/dyld_shared_cache_arm64" \
PLAYCOVER_REGRESSION_PROBE=session-initializer SDL_VIDEODRIVER=dummy SDL_AUDIODRIVER=dummy \
CARGO_TARGET_DIR=/home/ben/touchHLE-a64/target cargo test --lib --features a64 \
  a64::native_initializer_tests::actual_ipa_regression -- --exact --ignored --nocapture --test-threads=1
```

Look for the `PLAYCOVER_REGRESSION_BOUNDARY:` line. The `session-initializer`
probe matches the device's `--a64-cache-session-test`. On 2026-10-10 the device
and desktop boundaries were byte-identical, so the desktop harness is a good
first check before you spend time on a device run.

## Gotchas

- **Error popups hide the result.** Without `--no-error-popup`, an error opens
  the "Anastasis crashed" dialog, and `touchHLE errored:` never reaches logcat.
  The JSON then shows `runtime_error: null` while the process stays alive.
  The error text can only be read from a screenshot
  (`pixel-fold-tests/terraria-2026-10-10-plain-dialog.png`).
- On this debuggable build, Android shows an "Android App Compatibility / not
  16 KB aligned" dialog (libtouchHLE.so, libSDL2.so, libcarfile.so,
  libc++_shared.so). It takes focus, so the script skips its screenshot. It
  does not stop the run.
- The plain launch never reads the shared cache. On the normal path
  (`lib.rs` → `a64::run_file_with_reader`), ARM64-only apps load only
  `--a64-runtime=` files, so Terraria stops at Foundation. The cache is used
  only through the `--a64-cache-*` diagnostic options.
- `MainActivity` splits `extra_args` on spaces, so paths that contain spaces
  cannot be passed. `runtime_options` are filtered to an allow-list, so the
  diagnostic flags must go in `extra_args`.
- There are two adb servers (ports 5037 and 5038), and each lists the tablet
  twice (`... (2)`). Pass `--adb-port` explicitly and use the serial without
  `(2)`.
- Git Bash: `wsl bash /mnt/c/...` needs `MSYS_NO_PATHCONV=1`, and so do
  `adb shell` commands that contain `/sdcard` or `/data` paths. Write Python
  helpers to files instead of using heredocs.
- The earlier Terraria evidence (2026-10-07) used the iOS 16 cache on external
  storage (`/sdcard/Android/data/.../files/ios-runtime/cache`). The exact flag
  string from that run is not recorded. Private storage is used now.
- If the tablet is unplugged, `mStayOn` is false. Wake it before every run.

## Status on 2026-10-10 (Terraria 4.5.0, IPA SHA256 5cf22795...)

Tested source: local `trunk` at b6b6739f plus the uncommitted working tree.
This does **not** include playcover/main 07a05680, which was pushed earlier the
same day. That commit adds a general `0xffff_fffd` (mach_absolute_time) trap
decode, a fallback `<user data>/ios-runtime` runtime root for the plain path,
and lets `--a64-cache-*` / `--no-error-popup` through `runtime_options`.
Rerun this loop on a tree that contains it before acting on the results below.

| Run | Evidence | First failure |
| --- | --- | --- |
| Plain launch | `terraria-2026-10-10-plain.json/.log`, `-plain-dialog.png` | `Could not run ARM64 app: cannot load dynamic library /System/Library/Frameworks/Foundation.framework/Foundation ... needs a runtime root; use --a64-runtime=PATH` (dialog only; process stayed alive) |
| Session diagnostic, device PID 26381 | `terraria-2026-10-10-session.json/.log` | cache preparation passed (5 images, 64 cache deps, 5973 imports). Then, inside the original ObjC v1 mapped callback (753 headers): `initializer trap 4294967293 at 0x1c260d35c: Unsupported Mach identity trap 4294967293; lr=0x1c260d624 sp=0x22f8ba5b0` |
| Native harness | `terraria-native-2026-10-10.log` | identical boundary |

Trap `0xfffffffd` is Mach trap -3 (`mach_absolute_time`), zero-extended from
W16. `src/a64_execution_session.rs::trap_number` decodes it only for the
audited iOS 11 (15G77) stub at PC `0x18095bbe4`, and
`src/a64_mach_identity.rs` rejects it otherwise. Earlier boundaries were
`MemoryError(0) at 0x0, lr=0x1800c14b4` (tablet, 2026-10-07) and
`at 0x4, lr=0x1800c4610` (native, 2026-10-08). Today's run stops in the same
callback, but no `bounded callback progress sample` lines are logged before it.
