"""Copy complete IPAs between explicitly identified ADB devices, with SHA checks."""
import argparse
import datetime
import hashlib
import json
from pathlib import Path
import shlex
import subprocess
import tempfile
import uuid


def digest(path):
    result = hashlib.sha256()
    with path.open('rb') as source:
        for block in iter(lambda: source.read(1024 * 1024), b''):
            result.update(block)
    return result.hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', required=True)
    parser.add_argument('--destination', required=True)
    parser.add_argument('--source-device', required=True)
    parser.add_argument('--destination-device', required=True)
    parser.add_argument('--package', default='org.touchhle.android.a64test')
    parser.add_argument('--report', default='pixel-fold-tests/ipa-sync-fold-to-tablet.json')
    args = parser.parse_args()
    if args.source == args.destination:
        parser.error('Source and destination must differ')
    if not all(c.isalnum() or c in '._' for c in args.package):
        parser.error('Invalid package name')

    def adb(serial, *command, binary=False):
        return subprocess.check_output(['adb', '-s', serial, *command], timeout=600,
                                       text=not binary)

    def shell(serial, command):
        return adb(serial, 'shell', command).strip()

    identities = {}
    for serial, expected in [(args.source, args.source_device),
                             (args.destination, args.destination_device)]:
        actual = shell(serial, 'getprop ro.product.device')
        if actual != expected:
            raise RuntimeError(f'{serial}: expected {expected}, found {actual}')
        identities[serial] = dict(device=actual, model=shell(serial, 'getprop ro.product.model'))

    root = f'/sdcard/Android/data/{args.package}/files/touchHLE_apps'
    shell(args.destination, 'mkdir -p ' + shlex.quote(root))
    def listing(serial):
        command = 'find ' + shlex.quote(root) + " -maxdepth 1 -type f -name '*.ipa' -print0"
        return {p.rsplit('/', 1)[-1]: p for p in adb(serial, 'exec-out', command, binary=True)
                .decode('utf-8').split('\0') if p}

    def remote_digest(serial, path):
        value = shell(serial, 'sha256sum ' + shlex.quote(path)).split()[0]
        if len(value) != 64 or any(c not in '0123456789abcdef' for c in value):
            raise RuntimeError('Invalid remote SHA256 response')
        return value

    report = dict(started_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),
                  source=args.source, destination=args.destination, identities=identities,
                  files=[], complete=False)
    report_path = Path(args.report)
    report_path.parent.mkdir(parents=True, exist_ok=True)
    source_files, destination_files = listing(args.source), listing(args.destination)
    # Known local copies are reusable only after their bytes match the source.
    candidates = [p for p in Path('pixel-fold-tests').glob('*.ipa') if p.is_file()]
    local_by_digest = {digest(p): p for p in candidates}
    try:
        with tempfile.TemporaryDirectory(prefix='touchhle-ipa-sync-') as staging:
            for index, (name, source_path) in enumerate(sorted(source_files.items())):
                source_sha = remote_digest(args.source, source_path)
                destination_path = root + '/' + name
                previous_sha = (remote_digest(args.destination, destination_files[name])
                                if name in destination_files else None)
                entry = dict(name=name, source_sha256=source_sha, previous_sha256=previous_sha)
                if previous_sha == source_sha:
                    entry.update(action='identical', destination_sha256=previous_sha)
                else:
                    local = local_by_digest.get(source_sha)
                    if local is None:
                        local = Path(staging) / f'{index}.ipa'
                        adb(args.source, 'pull', source_path, str(local))
                    if digest(local) != source_sha:
                        raise RuntimeError(f'Local SHA256 mismatch for {name}')
                    pending = root + '/.ipa-sync-' + uuid.uuid4().hex
                    adb(args.destination, 'push', str(local), pending)
                    if remote_digest(args.destination, pending) != source_sha:
                        raise RuntimeError(f'Transferred SHA256 mismatch for {name}')
                    if previous_sha:
                        backup = destination_path + '.backup-' + uuid.uuid4().hex
                        shell(args.destination, 'mv ' + shlex.quote(destination_path) + ' ' + shlex.quote(backup))
                        entry['backup'] = backup
                    shell(args.destination, 'mv ' + shlex.quote(pending) + ' ' + shlex.quote(destination_path))
                    final_sha = remote_digest(args.destination, destination_path)
                    if final_sha != source_sha:
                        raise RuntimeError(f'Final SHA256 mismatch for {name}')
                    entry.update(action='copied', destination_sha256=final_sha)
                report['files'].append(entry)
                print(entry['action'], name, source_sha, flush=True)
                report_path.write_text(json.dumps(report, indent=2), encoding='utf-8')
        report['complete'] = True
        report['finished_at'] = datetime.datetime.now(datetime.timezone.utc).isoformat()
    finally:
        report_path.write_text(json.dumps(report, indent=2), encoding='utf-8')


if __name__ == '__main__':
    main()
