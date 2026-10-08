"""Transfer exact user-supplied IPAs and record normal device launch outcomes."""
import argparse
import datetime
import hashlib
import json
from pathlib import Path
import re
import shlex
import subprocess
import time
import uuid
from adb_device import discover_device

parser = argparse.ArgumentParser()
parser.add_argument('ipas', type=Path, nargs='+')
parser.add_argument('--reuse', action='store_true', help='Use the previously verified device copy')
parser.add_argument('--diagnostic', action='store_true', help='Send errors to logs instead of a blocking popup')
args = parser.parse_args()
serial = discover_device('comet')
package = 'org.touchhle.android.a64test'
folder = Path('pixel-fold-tests')
folder.mkdir(exist_ok=True)

def adb(*args, timeout=45):
    result = subprocess.run(['adb', '-s', serial, *args], capture_output=True, timeout=timeout)
    if result.returncode:
        raise RuntimeError(result.stderr.decode(errors='replace'))
    return result.stdout.decode(errors='replace').strip()

apk_digest = hashlib.sha256(Path('PlayCover-A-9Pro-test.apk').read_bytes()).hexdigest()
for source in args.ipas:
    name = re.sub(r'[^a-z0-9]+', '-', source.stem.lower()).strip('-')
    digest = hashlib.sha256(source.read_bytes()).hexdigest()
    staging = f'/data/local/tmp/playcover-{uuid.uuid4().hex}.ipa'
    relative = f'files/requested-tests/{name}.ipa'
    target = f'/data/user/0/{package}/{relative}'
    if not args.reuse:
        print(f'Transferring {source.name}', flush=True)
        adb('push', str(source), staging, timeout=900)
        try:
            adb('shell', 'run-as', package, 'mkdir', '-p', 'files/requested-tests')
            adb('shell', 'run-as', package, 'cp', staging, relative)
        finally:
            adb('shell', 'rm', staging)
    actual = adb('shell', 'run-as', package, 'sha256sum', relative).split()[0]
    if actual != digest:
        raise RuntimeError('IPA transfer checksum mismatch')
    adb('shell', 'am', 'force-stop', package)
    start = adb('shell', "date '+%m-%d %H:%M:%S.000'")
    extra = ' --es extra_args --no-error-popup' if args.diagnostic else ''
    launch = adb('shell', f'am start -n {package}/org.touchhle.android.MainActivity --es app_path {shlex.quote(target)}{extra}')
    time.sleep(15)
    log = adb('logcat', '-d', '-T', start, '-v', 'brief')
    log = '\n'.join(line for line in log.splitlines() if re.match(r'[VDIWEF]/(?:SDL/APP|touchHLE|AndroidRuntime|libc)', line))
    panic = re.search(r'Panic at[^\n]*', log)
    report = dict(tested_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),
                  serial=serial, package=package, source=str(source.resolve()),
                  device_ipa=target, ipa_sha256=digest, transferred_sha256=actual,
                  apk_sha256=apk_digest, launch=launch, log=log,
                  first_panic=panic.group(0) if panic else None,
                  diagnostic_only=args.diagnostic,
                  extra_args=['--no-error-popup'] if args.diagnostic else [],
                  app_functionality_verified=False)
    error = re.search(r'touchHLE errored:[^\n]*', log)
    report['runtime_error'] = error.group(0) if error else None
    suffix = 'diagnostic' if args.diagnostic else 'installed'
    (folder/f'{name}-{suffix}.json').write_text(json.dumps(report, indent=2))
    print(log[-6000:], flush=True)
