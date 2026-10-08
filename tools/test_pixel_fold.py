"""Run one explicit ARM64 IPA smoke test on the paired first-gen Pixel Fold."""
import argparse
import datetime
import hashlib
import json
import pathlib
import re
import shlex
import subprocess
import time
import uuid
from adb_device import discover_device

parser = argparse.ArgumentParser()
parser.add_argument("--serial")
parser.add_argument("--ipa", type=pathlib.Path, required=True)
parser.add_argument("--expected-exit", type=int, default=42)
parser.add_argument("--expected-error")
parser.add_argument("--report", type=pathlib.Path, required=True)
parser.add_argument("--package", default="org.touchhle.android")
parser.add_argument("--device", choices=["felix", "comet"], default="felix")
parser.add_argument("--guest-args", default="--headless")
parser.add_argument("--timeout-seconds", type=int, default=20)
args = parser.parse_args()
package = args.package
if not re.fullmatch(r"[a-zA-Z0-9_.]+", package):
    raise ValueError("Invalid Android package name")
args.serial = discover_device(args.device, args.serial)


def adb(*command, binary_input=None, timeout=30):
    result = subprocess.run(["adb", "-s", args.serial, *command],
                            input=binary_input, capture_output=True, timeout=timeout)
    if result.returncode:
        raise RuntimeError(result.stderr.decode(errors="replace"))
    return result.stdout.decode(errors="replace")


device = adb("shell", "getprop", "ro.product.device").strip()
if device != args.device:
    raise RuntimeError(f"Expected device {args.device}, got {device}")
model = adb("shell", "getprop", "ro.product.model").strip()
abi = adb("shell", "getprop", "ro.product.cpu.abilist").strip()
if "arm64-v8a" not in abi:
    raise RuntimeError(f"Device does not advertise arm64-v8a: {abi}")
adb("shell", "run-as", package, "mkdir", "-p", "files/a64-tests")
ipa_path = f"/data/user/0/{package}/files/a64-tests/test.ipa"
# ADB's push protocol preserves binary bytes on Windows; shell stdin does not.
staging = f"/data/local/tmp/playcover-a64-{uuid.uuid4().hex}.ipa"
adb("push", str(args.ipa), staging, timeout=90)
try:
    adb("shell", "run-as", package, "cp", staging, "files/a64-tests/test.ipa")
finally:
    adb("shell", "rm", staging)
expected_hash = hashlib.sha256(args.ipa.read_bytes()).hexdigest()
transferred_hash = adb("shell", "run-as", package, "sha256sum", "files/a64-tests/test.ipa").split()[0]
if transferred_hash != expected_hash:
    raise RuntimeError("IPA transfer checksum mismatch")
adb("shell", "am", "force-stop", package)
start = adb("shell", "date", "+%m-%d\\ %H:%M:%S.000").strip()
launch = adb("shell", "am", "start", "-n", f"{package}/org.touchhle.android.MainActivity",
             "--es", "app_path", ipa_path, "--es", "extra_args", shlex.quote(args.guest_args))
if "Error:" in launch:
    raise RuntimeError(launch)
expected = args.expected_error or f"ARM64 guest exited with code {args.expected_exit}"
output = ""
passed = False
for _ in range(args.timeout_seconds):
    time.sleep(1)
    output = adb("logcat", "-d", "-v", "brief", "-T", start,
                 "SDL:I", "SDL/APP:I", "touchHLE:I", "AndroidRuntime:E", "libc:F", "*:S")
    if expected in output:
        passed = True
        break
    if "Fatal signal" in output or "FATAL EXCEPTION" in output:
        break
report = {"tested_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
          "model": model, "device": device, "abi": abi,
          "ipa": str(args.ipa.resolve()), "expected": expected,
          "passed": passed, "launch": launch, "log": output}
args.report.parent.mkdir(parents=True, exist_ok=True)
args.report.write_text(json.dumps(report, indent=2))
print(json.dumps({k: v for k, v in report.items() if k not in ("log", "launch")}, indent=2))
if not passed:
    print(output[-5000:])
    raise SystemExit(1)
