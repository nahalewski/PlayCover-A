"""Stage the verified original iOS 11 cache in our side-by-side test package."""
import hashlib
import json
from pathlib import Path
import subprocess
import uuid
from adb_device import discover_device

serial = discover_device("comet")
package = "org.touchhle.android.a64test"
source = Path("ios-runtime/legacy-cache/System/Library/Caches/com.apple.dyld/dyld_shared_cache_arm64")
expected = "b196c0c60837bb9708605e6ef53a068bc44f1c2872847eb1f1e006e1b9361f84"
destination = "files/a64-cache-ios11/dyld_shared_cache_arm64"

def adb(*args, timeout=60):
    return subprocess.check_output(["adb", "-s", serial, *args], timeout=timeout, text=True).strip()

if adb("shell", "getprop", "ro.product.device") != "comet":
    raise RuntimeError("Expected Pixel 9 Pro Fold")
with source.open("rb") as file:
    if hashlib.file_digest(file, "sha256").hexdigest() != expected:
        raise RuntimeError("Source cache checksum mismatch")
adb("shell", "run-as", package, "mkdir", "-p", "files/a64-cache-ios11")
exists = subprocess.run(["adb", "-s", serial, "shell", "run-as", package, "sha256sum", destination], capture_output=True, text=True, timeout=60)
if exists.returncode or not exists.stdout.startswith(expected):
    staging = f"/data/local/tmp/playcover-cache-{uuid.uuid4().hex}"
    try:
        adb("push", str(source), staging, timeout=240)
        adb("shell", "run-as", package, "cp", staging, destination, timeout=120)
    finally:
        adb("shell", "rm", "-f", staging)
actual = adb("shell", "run-as", package, "sha256sum", destination).split()[0]
if actual != expected:
    raise RuntimeError("Device cache checksum mismatch")
report = {"serial": serial, "package": package, "cache_sha256": actual,
          "guest_cache_path": f"/data/user/0/{package}/{destination}", "verified": True,
          "apple_code_executed": False}
Path("pixel-fold-tests/9pro-legacy-cache-transfer.json").write_text(json.dumps(report, indent=2))
print(json.dumps(report, indent=2))
