"""Update one authorized device, preserve app data, and verify installed bytes."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

parser = argparse.ArgumentParser()
parser.add_argument('--serial', required=True)
parser.add_argument('--apk', default='PlayCover-A-9Pro-test.apk')
parser.add_argument('--label', default='latest-install')
args = parser.parse_args()
apk = Path(args.apk)
with apk.open('rb') as stream:
    expected = hashlib.file_digest(stream, 'sha256').hexdigest()
adb = ['adb', '-P', '5038', '-s', args.serial]
model = subprocess.check_output(adb + ['shell', 'getprop', 'ro.product.model'], text=True).strip()
subprocess.run(adb + ['install', '-r', str(apk)], check=True, timeout=600)
paths = subprocess.check_output(adb + ['shell', 'pm', 'path', 'org.touchhle.android.a64test'], text=True).splitlines()
base = next(line.removeprefix('package:') for line in paths if line.endswith('/base.apk'))
actual = subprocess.check_output(adb + ['shell', 'sha256sum', base], text=True).split()[0]
if actual != expected:
    raise RuntimeError('Installed APK hash differs from built APK')
result = dict(serial=args.serial, model=model, apk_sha256=actual, installed=True, data_preserved=True)
Path('pixel-fold-tests').mkdir(exist_ok=True)
Path('pixel-fold-tests', args.label + '.json').write_text(json.dumps(result, indent=2))
print(json.dumps(result), flush=True)
