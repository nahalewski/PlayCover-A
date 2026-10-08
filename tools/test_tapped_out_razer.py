"""Test the supplied Tapped Out IPA on the USB-connected Razer Edge."""
from pathlib import Path
import hashlib
import json
import plistlib
import re
import shlex
import struct
import subprocess
import time
import zipfile

source = Path(r'C:\Users\Ben\Downloads\The-Simpsons-Tapped-Out-v4-28-0.ipa')
serial = '602305N15309192'
package = 'org.touchhle.android.a64test'
folder = Path('pixel-fold-tests')
folder.mkdir(exist_ok=True)

def adb(*args, binary=False):
    return subprocess.check_output(['adb', '-P', '5038', '-s', serial, *args], timeout=120,
        **({} if binary else dict(text=True, encoding='utf-8', errors='replace')))

identity = {key: adb('shell', 'getprop', 'ro.product.' + key).strip()
            for key in ('device', 'model', 'manufacturer')}
assert identity['device'] == 'RZ45-0460' and 'Razer' in identity['model'], identity
with zipfile.ZipFile(source) as archive:
    plist_path = next(n for n in archive.namelist() if n.startswith('Payload/') and n.count('/') == 2 and n.endswith('/Info.plist'))
    info = plistlib.loads(archive.read(plist_path))
    executable = archive.read(plist_path.rsplit('/', 1)[0] + '/' + info['CFBundleExecutable'])
slices = []
assert executable[:4] == bytes.fromhex('cafebabe')
for i in range(struct.unpack_from('>I', executable, 4)[0]):
    cpu, subtype, offset, size, align = struct.unpack_from('>IIIII', executable, 8 + i * 20)
    thin = executable[offset:offset + size]
    is64 = cpu == 0x100000c
    position = 32 if is64 else 28
    cryptid = None
    for _ in range(struct.unpack_from('<I', thin, 16)[0]):
        cmd, length = struct.unpack_from('<II', thin, position)
        assert length >= 8 and position + length <= len(thin)
        if cmd in (0x21, 0x2c):
            cryptid = struct.unpack_from('<I', thin, position + 16)[0]
        position += length
    slices.append(dict(architecture='arm64' if is64 else 'armv7', cryptid=cryptid, bytes=size))
digest = hashlib.sha256(source.read_bytes()).hexdigest()
remote = f'/sdcard/Android/data/{package}/files/touchHLE_apps/{source.name}'
existing = subprocess.run(['adb', '-P', '5038', '-s', serial, 'shell', 'sha256sum ' + shlex.quote(remote)],
                          capture_output=True, text=True, encoding='utf-8', errors='replace')
if existing.returncode != 0 or not existing.stdout.startswith(digest):
    assert existing.returncode != 0, 'A different IPA already exists at the destination'
    staging = remote + '.testing-part'
    adb('push', str(source), staging)
    assert adb('shell', 'sha256sum ' + shlex.quote(staging)).split()[0] == digest
    adb('shell', 'mv ' + shlex.quote(staging) + ' ' + shlex.quote(remote))
installed = adb('shell', 'pm', 'path', package).splitlines()[0].removeprefix('package:')
apk_digest = adb('shell', 'sha256sum ' + shlex.quote(installed)).split()[0]
adb('shell', 'am', 'force-stop', package)
started = adb('shell', "date '+%m-%d %H:%M:%S.000'").strip()
launch = adb('shell', f'am start -n {package}/org.touchhle.android.MainActivity --es app_path {shlex.quote(remote)} --esa runtime_options --reported-ios-version=7.0')
time.sleep(20)
log = adb('logcat', '-d', '-T', started, '-v', 'brief', 'SDL/APP:I', 'SDL:I', 'touchHLE:I', 'AndroidRuntime:E', 'libc:F', '*:S')
(folder / 'tapped-out-razer.log').write_text(log, encoding='utf-8')
image = folder / 'tapped-out-razer.png'
image.write_bytes(adb('exec-out', 'screencap', '-p', binary=True))
focus = [s for s in adb('shell', 'dumpsys', 'window').splitlines() if 'mCurrentFocus=' in s]
panic = re.search(r'Panic at[^\n]*', log)
report = dict(identity=identity, ipa=str(source), ipa_sha256=digest, version=info.get('CFBundleShortVersionString'),
              minimum_ios=info.get('MinimumOSVersion'), slices=slices, apk_sha256=apk_digest,
              launch=launch, first_panic=panic.group(0) if panic else None, focus=focus,
              screenshot=str(image), gameplay_verified=False)
(folder / 'tapped-out-razer.json').write_text(json.dumps(report, indent=2), encoding='utf-8')
print(json.dumps(report, ensure_ascii=True))
print(log[-12000:])
