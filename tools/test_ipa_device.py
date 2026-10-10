"""Import and capture startup of an authorized IPA on an identified device."""
import argparse
import hashlib
import json
from pathlib import Path
import plistlib
import re
import shlex
import subprocess
import tempfile
import time
import zipfile
import uuid


def log_pid(line):
    """Accept Android threadtime and the historical brief evidence format."""
    match = re.match(r'\d{2}-\d{2}\s+\d{2}:\d{2}:\d{2}\.\d+\s+(\d+)\s+\d+\s+[VDIWEF]\s', line)
    if not match:
        match = re.match(r'[VDIWEF]/[^\n]*?\(\s*(\d+)\s*\):', line)
    return int(match.group(1)) if match else None


def power_evidence(setting, power):
    """Keep only power-policy fields, never retain a complete device dump."""
    result = {'stay_on_while_plugged_in': int(setting.strip()) if setting.strip().isdecimal() else None}
    for field in ('mStayOn', 'mWakefulness', 'mIsPowered', 'mPlugType'):
        match = re.search(r'^\s*' + field + r'=(true|false|Awake|Asleep|Dozing|Dreaming|\d+)\s*$', power, re.M)
        result[field] = match.group(1) if match else None
    return result


def scoped_outcome(log, marker, package, observed_pids=(), process_name=None):
    """Fail closed if the launch marker or unique process evidence is absent."""
    lines = log.splitlines()
    markers = [i for i, line in enumerate(lines) if marker in line]
    if len(markers) != 1:
        return dict(run_pid=None, outcome_scope_error='launch marker missing or duplicated',
                    first_panic=None, runtime_error=None)
    lines = lines[markers[0] + 1:]
    pids = {int(pid) for pid in observed_pids}
    # MainActivity runs in the manifest's exact :game process. Callers that
    # collect a main-process activity can explicitly supply its package name.
    process_name = process_name or package + ':game'
    expression = re.compile(r'\bStart proc (\d+):' + re.escape(process_name) + r'/')
    for line in lines:
        match = expression.search(line)
        if match:
            pids.add(int(match.group(1)))
    if len(pids) != 1:
        return dict(run_pid=None, outcome_scope_error='no unique launched process identity',
                    first_panic=None, runtime_error=None)
    pid = next(iter(pids))
    scoped = '\n'.join(line for line in lines if log_pid(line) == pid)
    panic = re.search(r'Panic at[^\n]*', scoped)
    error = re.search(r'touchHLE errored:[^\n]*', scoped)
    return dict(run_pid=pid, outcome_scope_error=None,
                first_panic=panic.group(0) if panic else None,
                runtime_error=error.group(0) if error else None)

def main():
    p = argparse.ArgumentParser()
    p.add_argument('--ipa', required=True)
    p.add_argument('--serial', required=True)
    p.add_argument('--device', required=True)
    p.add_argument('--adb-port', default='5038')
    p.add_argument('--label', required=True)
    p.add_argument('--runtime-option', action='append', default=[])
    p.add_argument('--extra-args', default='')
    p.add_argument('--wait-seconds', type=float, default=45)
    p.add_argument('--app-owned-copy', action='store_true', help='Stage a verified copy in the debuggable test app private files when external FUSE ownership prevents access')
    p.add_argument("--existing-private-only", action="store_true", help="Launch only a SHA-matching already staged private IPA; never copy or overwrite")
    a = p.parse_args()
    source = Path(a.ipa)
    package = 'org.touchhle.android'
    folder = Path('pixel-fold-tests')
    folder.mkdir(exist_ok=True)
    assert re.fullmatch(r'[a-z0-9-]+', a.label)

    def adb(*args, binary=False):
        return subprocess.check_output(['adb', '-P', a.adb_port, '-s', a.serial, *args], timeout=240,
            **({} if binary else dict(text=True, encoding='utf-8', errors='replace')))

    identity = {k: adb('shell', 'getprop', 'ro.product.' + k).strip() for k in ('device', 'model')}
    assert identity['device'] == a.device, identity
    with zipfile.ZipFile(source) as archive:
        n = next(n for n in archive.namelist() if n.startswith('Payload/') and n.count('/') == 2 and n.endswith('/Info.plist'))
        info = plistlib.loads(archive.read(n))
    with source.open('rb') as stream:
        digest = hashlib.file_digest(stream, 'sha256').hexdigest()
    remote = f'/sdcard/Android/data/{package}/files/touchHLE_apps/{source.name}'
    if not a.existing_private_only:
        existing = subprocess.run(['adb', '-P', a.adb_port, '-s', a.serial, 'shell', 'sha256sum ' + shlex.quote(remote)],
            capture_output=True, text=True, encoding='utf-8', errors='replace')
        if existing.returncode != 0:
            staging = remote + '.testing-part'
            partial = subprocess.run(['adb', '-P', a.adb_port, '-s', a.serial, 'shell', 'stat -c %s ' + shlex.quote(staging)],
                capture_output=True, text=True, encoding='utf-8', errors='replace')
            offset = int(partial.stdout.strip()) if partial.returncode == 0 else 0
            assert 0 <= offset <= source.stat().st_size
            if offset:
                h = hashlib.sha256()
                with source.open('rb') as stream:
                    remaining = offset
                    while remaining:
                        block = stream.read(min(8 * 1024 * 1024, remaining))
                        h.update(block)
                        remaining -= len(block)
                assert adb('shell', 'sha256sum ' + shlex.quote(staging)).split()[0] == h.hexdigest(), 'Partial file differs; preserving it'
            else:
                adb('shell', 'touch ' + shlex.quote(staging))
            print(f'Resuming verified IPA at {offset} / {source.stat().st_size} bytes', flush=True)
            chunk_remote = remote + '.testing-chunk'
            with tempfile.TemporaryDirectory(prefix='playcover-ipa-test-') as temporary:
                chunk_file = Path(temporary) / 'chunk'
                with source.open('rb') as stream:
                    stream.seek(offset)
                    while block := stream.read(8 * 1024 * 1024):
                        chunk_file.write_bytes(block)
                        adb('push', str(chunk_file), chunk_remote)
                        assert adb('shell', 'sha256sum ' + shlex.quote(chunk_remote)).split()[0] == hashlib.sha256(block).hexdigest()
                        adb('shell', 'cat ' + shlex.quote(chunk_remote) + ' >> ' + shlex.quote(staging) + '; rm ' + shlex.quote(chunk_remote))
                        offset += len(block)
                        print(f'Copied {offset} / {source.stat().st_size} bytes', flush=True)
            assert adb('shell', 'sha256sum ' + shlex.quote(staging)).split()[0] == digest
            adb('shell', 'mv ' + shlex.quote(staging) + ' ' + shlex.quote(remote))
        else:
            assert existing.stdout.startswith(digest), 'Different IPA exists; preserving it'
    if a.app_owned_copy or a.existing_private_only:
        private = 'files/touchHLE_device_tests/' + source.name
        check = subprocess.run(['adb', '-P', a.adb_port, '-s', a.serial, 'shell',
                                'run-as', package, 'sha256sum', private],
                               capture_output=True, text=True, timeout=240)
        if check.returncode != 0:
            if a.existing_private_only:
                raise RuntimeError("Required staged private IPA is absent; no files changed")
            destination = private + '.testing-part'
            command = ('mkdir -p files/touchHLE_device_tests && cat > ' + shlex.quote(destination))
            adb('shell', 'cat ' + shlex.quote(remote) + ' | run-as ' + shlex.quote(package) + ' sh -c ' + shlex.quote(command))
            assert adb('shell', 'run-as', package, 'sha256sum', destination).split()[0] == digest
            adb('shell', 'run-as', package, 'mv', destination, private)
        else:
            assert check.stdout.split()[0] == digest, 'Different private IPA exists; preserving it'
        remote = '/data/user/0/' + package + '/' + private
    installed = adb('shell', 'pm', 'path', package).splitlines()[0].removeprefix('package:')
    apk_digest = adb('shell', 'sha256sum ' + shlex.quote(installed)).split()[0]
    adb('shell', 'am', 'force-stop', package)
    started = adb('shell', "date '+%m-%d %H:%M:%S.000'").strip()
    marker = 'playcover-test-' + uuid.uuid4().hex
    adb('shell', 'log', '-p', 'i', '-t', 'PlayCoverTest', marker)
    minimum = info.get('MinimumOSVersion', '')
    runtime_options = list(a.runtime_option)
    if re.fullmatch(r'[0-9]{1,3}\.[0-9]{1,3}(?:\.[0-9]{1,3})?', minimum):
        runtime_options.append(f'--reported-ios-version={minimum}')
    assert all(',' not in item for item in runtime_options)
    option = ' --esa runtime_options ' + shlex.quote(','.join(runtime_options)) if runtime_options else ''
    diagnostic = ' --es extra_args ' + shlex.quote(a.extra_args) if a.extra_args else ''
    launch = adb('shell', f'am start -n {package}/org.touchhle.android.MainActivity --es app_path {shlex.quote(remote)}' + option + diagnostic)
    process_name = package + ':game'
    pid_result = subprocess.run(['adb', '-P', a.adb_port, '-s', a.serial, 'shell', 'pidof', process_name],
        capture_output=True, text=True, encoding='utf-8', errors='replace', timeout=30)
    observed_pids = [int(value) for value in pid_result.stdout.split() if value.isdecimal()] if pid_result.returncode == 0 else []
    time.sleep(max(0, min(a.wait_seconds, 60)))
    log = adb('logcat', '-d', '-T', started, '-v', 'threadtime', 'PlayCoverTest:I', 'ActivityManager:I', 'SDL/APP:I', 'SDL:I', 'touchHLE:I', 'AndroidRuntime:E', 'libc:F', '*:S')
    (folder / f'{a.label}.log').write_text(log, encoding='utf-8')
    focus = [line for line in adb('shell', 'dumpsys', 'window').splitlines() if 'mCurrentFocus=' in line]
    image = None
    screenshot_warning = None
    if any(package + '/' in line for line in focus):
        image = folder / f'{a.label}.png'
        pixels = adb('exec-out', 'screencap', '-p', binary=True)
        signature = b'\x89PNG\r\n\x1a\n'
        prefix = pixels.find(signature)
        if prefix < 0 or prefix > 1024:
            raise RuntimeError('screencap did not return a bounded PNG response')
        # Foldable Android can print a multiple-display warning before PNG bytes.
        screenshot_warning = pixels[:prefix].decode('utf-8', errors='replace') if prefix else None
        image.write_bytes(pixels[prefix:])
    outcome = scoped_outcome(log, marker, package, observed_pids, process_name)
    try:
        power = power_evidence(adb('shell', 'settings', 'get', 'global', 'stay_on_while_plugged_in'),
                               adb('shell', 'dumpsys', 'power'))
    except subprocess.SubprocessError:
        power = {'unavailable': True}
    report = dict(identity=identity, ipa=str(source), ipa_sha256=digest, apk_sha256=apk_digest,
        bundle=info.get('CFBundleIdentifier'), version=info.get('CFBundleShortVersionString'),
        minimum_ios=minimum, launch=launch, launch_marker=marker, **outcome,
        focus=focus, power=power, screenshot=str(image) if image else None, screenshot_warning=screenshot_warning,
        screenshot_skipped_reason=None if image else 'Test package not focused', gameplay_verified=False)
    (folder / f'{a.label}.json').write_text(json.dumps(report, indent=2), encoding='utf-8')
    print(json.dumps(report, ensure_ascii=True))
    print(log[-16000:])

if __name__ == "__main__":
    main()
