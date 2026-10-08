"""Capture the already imported, hash-verified modern RE4 on the authorized Tab."""
import hashlib
import json
from pathlib import Path
import subprocess
import time

serial = 'R52Y8066STA'
package = 'org.touchhle.android.a64test'
remote = '/sdcard/Android/data/' + package + '/files/touchHLE_apps/resident-evil-4-v1.0.5-iosvizor.ipa'
label = 're4-modern-tablet-init-fixed'
def adb(*args, binary=False):
    return subprocess.check_output(['adb', '-P', '5038', '-s', serial, *args], timeout=180,
        **({} if binary else dict(text=True, encoding='utf-8', errors='replace')))
assert adb('shell', 'getprop', 'ro.product.device').strip() == 'gts11uwifi'
digest = adb('shell', 'sha256sum', remote).split()[0]
assert digest == '4c1b8d45c5ff93862b455dae382f8fed2aa530abb2c1980fffa57b8f26558bb2'
apk = adb('shell', 'pm', 'path', package).strip().removeprefix('package:')
apk_hash = adb('shell', 'sha256sum', apk).split()[0]
assert apk_hash == '9001966f03e4faf8e3eea773ded70d9cb0074efa5c1616c035461e9566586723'
adb('shell', 'am', 'force-stop', package)
started = adb('shell', "date '+%m-%d %H:%M:%S.000'").strip()
launch = adb('shell', f'am start -n {package}/org.touchhle.android.MainActivity --es app_path {remote} --esa runtime_options --reported-ios-version=17.0')
time.sleep(40)
folder = Path('pixel-fold-tests')
log = adb('logcat', '-d', '-T', started, '-v', 'brief', 'SDL/APP:I', 'SDL:I', 'touchHLE:I', 'AndroidRuntime:E', 'libc:F', '*:S')
(folder / (label + '.log')).write_text(log, encoding='utf-8')
(folder / (label + '.png')).write_bytes(adb('exec-out', 'screencap', '-p', binary=True))
report = dict(serial=serial, adb_port=5038, ipa_sha256=digest, apk_sha256=apk_hash, launch=launch,
    bundle='jp.co.capcom.RE4US', minimum_ios='17.0', gameplay_verified=False,
    focus=[s for s in adb('shell', 'dumpsys', 'window').splitlines() if 'mCurrentFocus=' in s])
(folder / (label + '.json')).write_text(json.dumps(report, indent=2), encoding='utf-8')
print(json.dumps(report))
print(log[-12000:])
