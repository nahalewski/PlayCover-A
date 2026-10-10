"""Stage an original ARM64 dyld cache and/or IPAs into the app's private files.

Run from the workspace parent (the folder that holds ios-runtime/ and
pixel-fold-tests/). Every file is SHA256-verified locally, after the push to
/data/local/tmp, and again after it lands in app-private storage. An existing
private file with the same hash is kept; a different existing file aborts the
run and is never overwritten. Reruns are idempotent and resume per file.

Example (iOS 16 cache + Terraria on the Tab S11):
  python touchHLE-src/tools/stage_a64_device.py --serial SERIAL --device gts11uwifi \
      --cache-manifest pixel-fold-tests/tablet-ios16-runtime.json \
      --cache-source ios-runtime/cache/System/Library/Caches/com.apple.dyld \
      --cache-dest ios-runtime/cache \
      --ipa C:/Users/Ben/Desktop/ipa/Terraria_4.5.0.ipa \
      --report pixel-fold-tests/terraria-tablet-stage-2026-10-10.json
"""
import argparse
import hashlib
import json
from pathlib import Path
import shlex
import subprocess
import time


def sha256_file(path):
    with open(path, 'rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def main():
    p = argparse.ArgumentParser()
    p.add_argument('--serial', required=True)
    p.add_argument('--device', required=True, help='Expected ro.product.device, e.g. gts11uwifi')
    p.add_argument('--adb-port', default='5037')
    p.add_argument('--package', default='org.touchhle.android')
    p.add_argument('--cache-manifest', type=Path, help='JSON with files[{file,bytes,sha256}] (e.g. tablet-ios16-runtime.json)')
    p.add_argument('--cache-source', type=Path, help='Local directory holding the cache files named in the manifest')
    p.add_argument('--cache-dest', default='ios-runtime/cache', help='Directory relative to the app files/ dir')
    p.add_argument('--ipa', type=Path, action='append', default=[])
    p.add_argument('--ipa-dest', default='touchHLE_device_tests', help='Directory relative to the app files/ dir')
    p.add_argument('--report', type=Path, required=True)
    a = p.parse_args()
    prefix = ['adb', '-P', a.adb_port, '-s', a.serial]

    def adb(*args, timeout=600):
        return subprocess.check_output(prefix + list(args), text=True, timeout=timeout,
                                       encoding='utf-8', errors='replace').strip()

    def run(*args):
        return subprocess.run(prefix + list(args), text=True, capture_output=True,
                              encoding='utf-8', errors='replace', timeout=600)

    assert adb('shell', 'getprop', 'ro.product.device') == a.device, 'Unexpected device'
    adb('shell', 'run-as', a.package, 'true')  # fails unless the package is installed and debuggable

    plan = []  # (local path, private relative path, expected sha256, bytes)
    if a.cache_manifest:
        assert a.cache_source, '--cache-source is required with --cache-manifest'
        manifest = json.loads(a.cache_manifest.read_text())
        for record in manifest['files']:
            plan.append((a.cache_source / record['file'], a.cache_dest + '/' + record['file'],
                         record['sha256'], record['bytes']))
    for ipa in a.ipa:
        plan.append((ipa, a.ipa_dest + '/' + ipa.name, None, ipa.stat().st_size))

    base = '/data/user/0/' + a.package + '/files/'
    report = dict(serial=a.serial, device=a.device, package=a.package, files=[], complete=False,
                  started=time.strftime('%Y-%m-%dT%H:%M:%S'))
    started = time.monotonic()
    for index, (local, relative, expected, size) in enumerate(plan, 1):
        t0 = time.monotonic()
        assert local.stat().st_size == size, f'{local}: size differs from manifest'
        digest = sha256_file(local)
        if expected:
            assert digest == expected, f'{local}: local SHA256 differs from manifest; stopping'
        private = 'files/' + relative
        check = run('shell', 'run-as', a.package, 'sha256sum', private)
        if check.returncode == 0 and check.stdout.split():
            if check.stdout.split()[0] != digest:
                raise RuntimeError('Different existing private file preserved: ' + private)
            action = 'identical-existing'
        else:
            temporary = '/data/local/tmp/a64stage-' + local.name
            existing = run('shell', 'sha256sum', temporary)
            if not (existing.returncode == 0 and existing.stdout.split()[:1] == [digest]):
                adb('push', str(local), temporary, timeout=7200)
                assert adb('shell', 'sha256sum', temporary).split()[0] == digest, 'Push corrupted ' + temporary
            part = private + '.verified-staging'
            directory = private.rsplit('/', 1)[0]
            command = 'mkdir -p ' + shlex.quote(directory) + ' && cat > ' + shlex.quote(part)
            adb('shell', 'cat ' + shlex.quote(temporary) + ' | run-as ' + a.package + ' sh -c ' + shlex.quote(command),
                timeout=3600)
            assert adb('shell', 'run-as', a.package, 'sha256sum', part).split()[0] == digest
            adb('shell', 'run-as', a.package, 'mv', part, private)
            adb('shell', 'rm', temporary)
            action = 'copied-verified'
        seconds = round(time.monotonic() - t0, 1)
        report['files'].append(dict(source=str(local), path=base + relative, sha256=digest, bytes=size,
                                    action=action, seconds=seconds))
        print(f'{index}/{len(plan)} {action} {relative} ({size} bytes, {seconds}s)', flush=True)
        a.report.write_text(json.dumps(report, indent=2) + '\n')
    report['complete'] = True
    report['total_seconds'] = round(time.monotonic() - started, 1)
    a.report.write_text(json.dumps(report, indent=2) + '\n')
    print('complete in', report['total_seconds'], 's')


if __name__ == '__main__':
    main()
