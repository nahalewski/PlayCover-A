"""Inspect/test the exact installed Rock Band IPA on the authorized Tab S11."""
import argparse, datetime, hashlib, json, plistlib, re, shlex, struct, subprocess, time
from pathlib import Path

p = argparse.ArgumentParser()
p.add_argument('--inspect-only', action='store_true')
args = p.parse_args()
serial = '192.168.0.56:45601'
package = 'org.touchhle.android.a64test'
ipa = f'/sdcard/Android/data/{package}/files/touchHLE_apps/Rock Band 1.1.38.ipa'
folder = Path('pixel-fold-tests')
report_path = folder / 'rockband-gts11uwifi.json'

def adb(*words, binary=False):
    result = subprocess.check_output(['adb', '-s', serial, *words], timeout=60)
    return result if binary else result.decode('utf-8', errors='replace').strip()

report = dict(tested_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),
              serial=serial, device=adb('shell', 'getprop', 'ro.product.device'),
              model=adb('shell', 'getprop', 'ro.product.model'), ipa=ipa,
              gameplay_verified=False, menus_verified=False, inspect_only=args.inspect_only)
assert report['device'] == 'gts11uwifi'
report['ipa_sha256'] = adb('shell', 'sha256sum ' + shlex.quote(ipa)).split()[0]
assert report['ipa_sha256'] == '597355b1d09c70b6524cb2b5e9837568a6ba086c56cb830dd8b1ed62573baf11'
installed = adb('shell', 'pm', 'path', package).splitlines()[0].removeprefix('package:')
report['apk_sha256'] = adb('shell', 'sha256sum ' + shlex.quote(installed)).split()[0]
assert report['apk_sha256'] == hashlib.sha256(Path('PlayCover-A-fold-test.apk').read_bytes()).hexdigest(), 'Installed APK differs from current build'
plist_bytes = adb('exec-out', 'unzip -p ' + shlex.quote(ipa) + ' Payload/rockband.app/Info.plist', binary=True)
info = plistlib.loads(plist_bytes)
report['bundle'] = {k: info.get(k) for k in ['CFBundleDisplayName', 'CFBundleIdentifier', 'CFBundleExecutable',
                                          'CFBundleShortVersionString', 'CFBundleVersion', 'MinimumOSVersion']}
executable = 'Payload/rockband.app/' + info['CFBundleExecutable']
header = adb('exec-out', 'unzip -p ' + shlex.quote(ipa) + ' ' + shlex.quote(executable) + ' | head -c 1048576', binary=True)
magic, cpu, subtype, filetype, count, size, flags = struct.unpack_from('<7I', header)
assert magic in (0xfeedface, 0xfeedfacf), hex(magic)
start = 32 if magic == 0xfeedfacf else 28
assert count <= 4096 and size <= 1048576 and start + size <= len(header)
cursor = start; encrypted = []; dependencies = []
for _ in range(count):
    command, length = struct.unpack_from('<II', header, cursor)
    assert length >= 8 and cursor + length <= start + size
    if command in (0x21, 0x2c): encrypted.append(struct.unpack_from('<I', header, cursor + 16)[0])
    if command in (0xc, 0x80000018, 0x8000001f, 0x80000023):
        offset = struct.unpack_from('<I', header, cursor + 8)[0]
        dependencies.append(header[cursor + offset:cursor + length].split(b'\0')[0].decode())
    cursor += length
assert cursor == start + size
report['macho'] = dict(cpu_type=cpu, cpu_subtype=subtype, is_64_bit=magic == 0xfeedfacf,
                       encryption_cryptids=encrypted, dependencies=dependencies)
if not args.inspect_only:
    # Preserve the launcher/download service; stop only an existing game process.
    previous_game = subprocess.run(['adb', '-s', serial, 'shell', 'pidof', package + ':game'],
                                   capture_output=True, text=True, timeout=30).stdout.strip()
    for pid in previous_game.split():
        assert pid.isdigit()
        adb('shell', 'run-as', package, 'kill', '-9', pid)
    began = adb('shell', "date '+%m-%d %H:%M:%S.000'")
    report['launch'] = adb('shell', 'am start -n ' + package + '/org.touchhle.android.MainActivity --es app_path ' + shlex.quote(ipa))
    time.sleep(20)
    log = adb('logcat', '-d', '-T', began, '-v', 'brief', 'SDL/APP:I', 'SDL:I', 'touchHLE:I', 'AndroidRuntime:E', 'libc:F', '*:S')
    report['log'] = log
    report['focus'] = [line for line in adb('shell', 'dumpsys', 'window').splitlines() if 'mCurrentFocus=' in line]
    panic = re.search(r'Panic at[^\n]*', log)
    report['first_panic'] = panic.group(0) if panic else None
    report['native_crash'] = bool('Fatal signal' in log or 'FATAL EXCEPTION' in log)
    report['cpu_started'] = 'CPU emulation begins now' in log
    if any(package in line for line in report['focus']):
        remote = '/data/local/tmp/rockband-startup.png'
        adb('shell', 'screencap', '-p', remote)
        adb('pull', remote, str(folder / 'rockband-gts11uwifi.png'))
        report['screenshot'] = str(folder / 'rockband-gts11uwifi.png')
if report_path.exists():
    previous = json.loads(report_path.read_text(encoding='utf-8'))
    archived = folder / ('rockband-gts11uwifi-' + previous['apk_sha256'][:12] + '.json')
    archived.write_text(json.dumps(previous, indent=2), encoding='utf-8')
report_path.write_text(json.dumps(report, indent=2), encoding='utf-8')
print(json.dumps({k: v for k, v in report.items() if k != 'log'}, indent=2))
