import hashlib
import argparse
import json
import pathlib
import subprocess
import time
import shlex
from adb_device import discover_device

parser = argparse.ArgumentParser()
parser.add_argument('--serial')
parser.add_argument('--cache-prepare')
parser.add_argument('--expected', default='needs a runtime root; use --a64-runtime=PATH')
parser.add_argument('--report', default='pixel-fold-tests/pokemon-quest-9pro.json')
parser.add_argument('--timeout-seconds', type=int, default=45)
options = parser.parse_args()

SERIAL = discover_device('comet', options.serial)
PACKAGE = 'org.touchhle.android.a64test'

def adb(*args):
    return subprocess.run(['adb', '-s', SERIAL, *args], capture_output=True,
                          text=True, encoding='utf-8', errors='replace', check=True).stdout

model = adb('shell', 'getprop ro.product.model').strip()
device = adb('shell', 'getprop ro.product.device').strip()
assert device == 'comet', (model, device)
target = f'/data/user/0/{PACKAGE}/files/a64-tests/PokemonQuest.ipa'
stage = '/data/local/tmp/playcover-pokemonquest-test.ipa'
adb('shell', f"cp '/sdcard/Download/Pokemon Quest.ipa' {stage}")
adb('shell', f'chmod 644 {stage}')
adb('shell', f'run-as {PACKAGE} mkdir -p files/a64-tests')
adb('shell', f'run-as {PACKAGE} cp {stage} files/a64-tests/PokemonQuest.ipa')
adb('shell', f'rm {stage}')
remote_hash = adb('shell', f'run-as {PACKAGE} sha256sum files/a64-tests/PokemonQuest.ipa').split()[0]
with open('PokemonQuest-device.ipa', 'rb') as f:
    local_hash = hashlib.file_digest(f, 'sha256').hexdigest()
assert local_hash == remote_hash
adb('shell', f'am force-stop {PACKAGE}')
start = adb('shell', "date '+%m-%d %H:%M:%S.000'").strip()
guest_args = '--headless'
if options.cache_prepare:
    guest_args += ' --a64-cache-prepare=' + options.cache_prepare
launch = adb('shell', f'am start -n {PACKAGE}/org.touchhle.android.MainActivity --es app_path {target} --es extra_args {shlex.quote(guest_args)}')
expected = options.expected
log = ''
for _ in range(options.timeout_seconds):
    time.sleep(1)
    full = adb('logcat', '-d', '-T', start, '-v', 'brief')
    log = '\n'.join(line for line in full.splitlines() if line.startswith(('I/SDL/APP', 'E/AndroidRuntime', 'F/libc')))
    if expected in log or 'FATAL EXCEPTION' in log:
        break
report = dict(model=model, device=device, ipa=target, sha256=local_hash,
              launch=launch, expected=expected, blocker_confirmed=expected in log,
              legacy_format_blocker_removed=expected in log,
              game_runs=False, log=log)
pathlib.Path(options.report).write_text(json.dumps(report, indent=2), encoding='utf-8')
print(json.dumps({k:v for k,v in report.items() if k not in ('log', 'launch')}, indent=2))
print(log[-6000:])
if expected not in log:
    raise SystemExit(1)
