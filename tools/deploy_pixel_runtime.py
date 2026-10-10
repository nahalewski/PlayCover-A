"""Stage unchanged original ARM64 iOS 16.7 cache on the paired Pixel Fold, SHA verified."""
from pathlib import Path
import subprocess
import hashlib
import json
import sys

source = Path("ios-runtime/cache")
remote = "/sdcard/Android/data/org.touchhle.android.a64test/files/ios-runtime/cache"
serial = "192.168.0.51:35429"

def adb(*args, timeout=300):
    return subprocess.check_output(["adb", "-s", serial, *args], timeout=timeout, text=True)

device = adb("shell", "getprop", "ro.product.device").strip()
if device != "felix":
    raise RuntimeError(f"Expected Pixel Fold (felix), got {device}")

print("Pixel Fold confirmed, checking destination storage...", flush=True)
print(adb("shell", "df", "-h", "/sdcard"), flush=True)
adb("shell", "mkdir", "-p", remote)

files = sorted(source.glob("dyld_shared_cache_arm64*"))
if len(files) != 44:
    raise RuntimeError(f"Expected 44 cache files, found {len(files)}")

records = []
total_bytes = sum(p.stat().st_size for p in files)
transferred_bytes = 0

print(f"Staging {len(files)} cache files ({total_bytes / (1024**3):.2f} GB) to {remote}...", flush=True)

for i, p in enumerate(files, 1):
    with p.open("rb") as stream:
        digest = hashlib.file_digest(stream, "sha256").hexdigest()
    dest = f"{remote}/{p.name}"
    check = subprocess.run(["adb", "-s", serial, "shell", f"sha256sum {dest}"],
                           capture_output=True, text=True)
    if check.returncode == 0 and check.stdout.split()[0] == digest:
        print(f"[{i}/44] Already verified: {p.name}", flush=True)
    else:
        part = f"{dest}.part"
        subprocess.check_call(["adb", "-s", serial, "push", str(p), part])
        part_check = adb("shell", f"sha256sum {part}").split()[0]
        if part_check != digest:
            raise RuntimeError(f"Checksum mismatch after pushing {p.name}")
        adb("shell", f"mv {part} {dest}")
        print(f"[{i}/44] Pushed and verified: {p.name} ({p.stat().st_size / (1024**2):.1f} MB)", flush=True)
    records.append(dict(file=p.name, bytes=p.stat().st_size, sha256=digest))

report = dict(remote=remote, files=records, execution_verified=False)
Path("pixel-fold-tests/felix-ios16-runtime.json").write_text(json.dumps(report, indent=2))
print("All 44 cache files successfully staged on Pixel Fold!", flush=True)
