"""Launch the user's existing Pokédex IPA without changing its library copy."""
import datetime
import json
import hashlib
import re
from pathlib import Path
import shlex
import subprocess
import time
from adb_device import discover_device

serial = discover_device('comet')
package = 'org.touchhle.android.a64test'
ipa = f'/storage/emulated/0/Android/data/{package}/files/touchHLE_apps/Pokedex.Plus v2.0.ipa'

def adb(*args, timeout=30):
    return subprocess.check_output(['adb', '-s', serial, *args], timeout=timeout, text=True, encoding='utf-8', errors='replace').strip()

digest = adb('shell', f'sha256sum {shlex.quote(ipa)}').split()[0]
adb('shell', 'am', 'force-stop', package)
start = adb('shell', "date '+%m-%d %H:%M:%S.000'")
launch = adb('shell', f'am start -n {package}/org.touchhle.android.MainActivity --es app_path {shlex.quote(ipa)}')
time.sleep(12)
log = adb('logcat', '-d', '-T', start, '-v', 'brief')
log = '\n'.join(line for line in log.splitlines() if line.startswith(('I/SDL/APP', 'E/AndroidRuntime', 'F/libc')))
folder = Path('pixel-fold-tests')
folder.mkdir(exist_ok=True)
screenshot = None
if 'Panic at' not in log and 'Fatal signal' not in log:
    foreground = adb('shell', 'dumpsys', 'window')
    focus = [line for line in foreground.splitlines() if 'mCurrentFocus=' in line]
    if any(package in line for line in focus):
        remote = '/data/local/tmp/playcover-pokedex-test.png'
        adb('shell', 'screencap', '-p', remote)
        adb('pull', remote, str(folder / 'pokedex-installed.png'))
        adb('shell', 'rm', remote)
        screenshot = 'pixel-fold-tests/pokedex-installed.png'
report = dict(tested_at=datetime.datetime.now(datetime.timezone.utc).isoformat(), serial=serial,
              package=package, ipa=ipa, ipa_sha256=digest, launch=launch, log=log,
              screenshot=screenshot)
report['apk_sha256'] = hashlib.sha256(Path('PlayCover-A-9Pro-test.apk').read_bytes()).hexdigest()
report['app'] = 'Pokedex Plus 2.0'
report['architecture'] = 'armv7 (32-bit)'
report['startup_survived'] = 'Panic at' not in log and 'Fatal signal' not in log
report['screen_verified'] = False
panic = re.search(r'Panic at[^\n]*', log)
report['first_fatal_blocker'] = panic.group(0) if panic else None
(folder / 'pokedex-installed.json').write_text(json.dumps(report, indent=2))
print(log[-9000:])
