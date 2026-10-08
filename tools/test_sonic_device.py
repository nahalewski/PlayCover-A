"""Capture Sonic startup evidence on an explicitly authorized ADB device."""
import argparse
import datetime
import hashlib
import json
from pathlib import Path
import re
import shlex
import subprocess
import time
from adb_device import discover_device

parser = argparse.ArgumentParser()
parser.add_argument('--serial')
parser.add_argument('--device', default='felix')
parser.add_argument('--game', choices=['sonic1', 'sonic2', 'both'], default='both')
args = parser.parse_args()
serial = args.serial or discover_device(args.device)
package = 'org.touchhle.android.a64test'
folder = Path('pixel-fold-tests')
folder.mkdir(exist_ok=True)

def adb(*args):
    return subprocess.check_output(['adb', '-s', serial, *args], timeout=60,
                                   text=True, encoding='utf-8', errors='replace').strip()

model = adb('shell', 'getprop', 'ro.product.model')
device = adb('shell', 'getprop', 'ro.product.device')
assert device == args.device, (device, args.device)
apk_digest = hashlib.sha256(Path('PlayCover-A-fold-test.apk').read_bytes()).hexdigest()
installed_path = adb('shell', 'pm', 'path', package).splitlines()[0].removeprefix('package:')
assert adb('shell', 'sha256sum ' + shlex.quote(installed_path)).split()[0] == apk_digest, 'Installed APK differs from build'
for filename, label in [('Sonic 1 1.2.6.ipa', 'sonic1'), ('Sonic 2 1.2.2.ipa', 'sonic2')]:
    if args.game != 'both' and args.game != label:
        continue
    ipa = f'/sdcard/Android/data/{package}/files/touchHLE_apps/{filename}'
    digest = adb('shell', 'sha256sum ' + shlex.quote(ipa)).split()[0]
    adb('shell', 'am', 'force-stop', package)
    start = adb('shell', "date '+%m-%d %H:%M:%S.000'")
    launch = adb('shell', f'am start -n {package}/org.touchhle.android.MainActivity --es app_path {shlex.quote(ipa)}')
    time.sleep(15)
    log = adb('logcat', '-d', '-T', start, '-v', 'brief', 'SDL/APP:I', 'SDL:I', 'touchHLE:I', 'AndroidRuntime:E', 'libc:F', '*:S')
    focus = [line for line in adb('shell', 'dumpsys', 'window').splitlines() if 'mCurrentFocus=' in line]
    image = None
    if any(package in line or 'PlayCover-A crashed' in line for line in focus):
        remote = '/data/local/tmp/sonic-startup.png'
        adb('shell', 'screencap', '-p', remote)
        image = folder / f'{label}-{device}.png'
        adb('pull', remote, str(image))
    panic = re.search(r'Panic at[^\n]*', log)
    report = dict(tested_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),
                  serial=serial, device=device, model=model, ipa=ipa, ipa_sha256=digest,
                  apk_sha256=apk_digest,
                  launch=launch, log=log, focus=focus, screenshot=str(image) if image else None,
                  first_panic=panic.group(0) if panic else None, gameplay_verified=False)
    report_path = folder / f'{label}-{device}.json'
    if report_path.exists():
        previous = json.loads(report_path.read_text(encoding='utf-8'))
        (folder / f'{label}-{device}-{previous["apk_sha256"][:12]}.json').write_text(json.dumps(previous, indent=2), encoding='utf-8')
    report_path.write_text(json.dumps(report, indent=2), encoding='utf-8')
    print(label, report['first_panic'], log[-8000:])
