"""Capture only Resident Evil 4 startup on a verified Razer Edge."""
import argparse
import datetime
import hashlib
import json
from pathlib import Path
import re
import shlex
import subprocess
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--serial', required=True)
    parser.add_argument('--device', required=True)
    parser.add_argument('--apk', default='PlayCover-A-fold-test.apk')
    parser.add_argument('--adb-port', type=int, default=5037)
    args = parser.parse_args()
    package = 'org.touchhle.android.a64test'
    def adb(*command, binary=False):
        return subprocess.check_output(['adb', '-P', str(args.adb_port), '-s', args.serial, *command], timeout=90,
                                       **({} if binary else dict(text=True, encoding='utf-8', errors='replace')))
    def prop(name):
        return adb('shell', 'getprop', name).strip()
    identity = {key: prop('ro.product.' + key) for key in ['device', 'model', 'manufacturer', 'brand']}
    description = ' '.join(identity.values()).lower()
    if identity['device'] != args.device or 'razer' not in description or 'edge' not in description:
        raise RuntimeError(f'Unexpected device: {identity}')
    apk_sha = hashlib.sha256(Path(args.apk).read_bytes()).hexdigest()
    installed = adb('shell', 'pm', 'path', package).splitlines()[0].removeprefix('package:')
    if adb('shell', 'sha256sum ' + shlex.quote(installed)).split()[0] != apk_sha:
        raise RuntimeError('Installed APK differs from requested build')
    ipa = f'/sdcard/Android/data/{package}/files/touchHLE_apps/Resident.Evil.4.HD.v1.0.ipa.ipa'
    ipa_sha = adb('shell', 'sha256sum ' + shlex.quote(ipa)).split()[0]
    adb('shell', 'am', 'force-stop', package)
    started = adb('shell', "date '+%m-%d %H:%M:%S.000'").strip()
    launch = adb('shell', f'am start -n {package}/org.touchhle.android.MainActivity --es app_path {shlex.quote(ipa)}')
    time.sleep(15)
    log = adb('logcat', '-d', '-T', started, '-v', 'brief', 'SDL/APP:I', 'SDL:I', 'AndroidRuntime:E', 'libc:F', '*:S')
    focus = [line for line in adb('shell', 'dumpsys', 'window').splitlines() if 'mCurrentFocus=' in line]
    folder = Path('pixel-fold-tests')
    folder.mkdir(exist_ok=True)
    screenshot = None
    if any(package in line or 'PlayCover-A crashed' in line for line in focus):
        screenshot = folder / 'resident-evil4-razer.png'
        screenshot.write_bytes(adb('exec-out', 'screencap', '-p', binary=True))
    panic = re.search(r'Panic at[^\n]*', log)
    report = dict(tested_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),
                  serial=args.serial, identity=identity, apk_sha256=apk_sha,
                  ipa=ipa, ipa_sha256=ipa_sha, launch=launch, focus=focus, log=log,
                  screenshot=str(screenshot) if screenshot else None,
                  first_panic=panic.group(0) if panic else None,
                  gameplay_verified=False, audio_verified=False)
    destination = folder / 'resident-evil4-razer.json'
    if destination.exists():
        prior = json.loads(destination.read_text(encoding='utf-8'))
        (folder / f'resident-evil4-razer-{prior["apk_sha256"][:12]}.json').write_text(json.dumps(prior, indent=2), encoding='utf-8')
    destination.write_text(json.dumps(report, indent=2), encoding='utf-8')
    print(json.dumps(dict(report=str(destination), first_panic=report['first_panic'], focus=focus)))


if __name__ == '__main__':
    main()
