#!/usr/bin/env python3
"""WSL: batch actual IPA dependency/libSystem initializer probes on a frozen build.

Run ipa_inventory.py first. Build once with cargo test --lib --features a64
--no-run --message-format=json and pass its stdout file as --build-json.
No app-main/gameplay success is inferred from this diagnostic collector.
"""
import argparse
from collections import defaultdict
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import time


def hashes(native):
    paths = [p for p in (native / 'src').rglob('*') if p.is_file()]
    paths += [p for p in (native / 'tests/a64').rglob('*') if p.is_file()]
    paths += [native / name for name in ('Cargo.toml', 'Cargo.lock', 'build.rs') if (native / name).is_file()]
    return {str(p.relative_to(native)): hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(paths)}


def file_digest(path):
    digest = hashlib.sha256()
    with Path(path).open('rb') as source:
        for block in iter(lambda: source.read(1024 * 1024), b''):
            digest.update(block)
    return digest.hexdigest()


def validate_build_proof(proof, source_hashes, executable, build_json):
    if proof.get('schema') != 1 or proof.get('build_succeeded') is not True:
        raise ValueError('Build proof does not record a successful build')
    before, after = proof.get('source_hashes_before'), proof.get('source_hashes_after')
    if not before or before != after:
        raise ValueError('Build proof source changed during build')
    if before != source_hashes:
        raise ValueError('Build proof source differs from current native snapshot')
    if proof.get('test_binary_sha256') != file_digest(executable):
        raise ValueError('Build proof binary hash mismatch')
    if proof.get('build_json_sha256') != file_digest(build_json):
        raise ValueError('Build proof Cargo JSON hash mismatch')


def fingerprint(boundary):
    # Addresses and bundle names vary; retain symbols/provider names/error text.
    value = re.sub(r'0x[0-9a-fA-F]+', '<address>', boundary)
    value = re.sub(r'/[^ /|]+\.app/', '/<app>.app/', value)
    value = re.sub(r'after \d+ supervisor traps', 'after <count> supervisor traps', value)
    value = re.sub(r'\d+ actual TLV images / \d+ descriptors / \d+ TLS initializer callbacks', '<count> actual TLV images / <count> descriptors / <count> TLS initializer callbacks', value)
    value = re.sub(r'for \d+ images/\d+ descriptors', 'for <count> images/<count> descriptors', value)
    return value


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--inventory', type=Path, required=True)
    parser.add_argument('--build-json', type=Path, required=True)
    parser.add_argument('--native', type=Path, default=Path('/home/ben/touchHLE-a64-integration'))
    parser.add_argument('--cache', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--timeout', type=int, default=120)
    parser.add_argument('--probe', choices=('initializer', 'legacy-cpp-prepare', 'session-initializer', 'session-image-info'), default='initializer')
    parser.add_argument('--build-proof', type=Path, help='Receipt from build_arm64_proof.py')
    parser.add_argument('--require-build-proof', action='store_true', help='Reject unverified artifact/source association')
    args = parser.parse_args()
    if args.timeout <= 0:
        parser.error('timeout must be positive')
    artifacts = []
    build_succeeded = False
    for line in args.build_json.read_text().splitlines():
        try:
            item = json.loads(line)
        except ValueError:
            continue
        if item.get('reason') == 'build-finished':
            build_succeeded = item.get('success') is True
        if item.get('reason') == 'compiler-artifact' and item.get('profile', {}).get('test') and item.get('executable') and set(item.get('target', {}).get('kind', [])) & {'lib', 'rlib', 'cdylib'}:
            artifacts.append(item['executable'])
    if not build_succeeded or len(set(artifacts)) != 1:
        raise SystemExit('Expected one successfully built library test executable')
    executable = artifacts[-1]
    args.cache = args.cache.resolve(strict=True)
    args.output.mkdir(parents=True, exist_ok=True)
    baseline = hashes(args.native)
    if args.require_build_proof and not args.build_proof:
        raise SystemExit('--require-build-proof requires --build-proof')
    if args.build_proof:
        validate_build_proof(json.loads(args.build_proof.read_text()), baseline, executable, args.build_json)
    report = {'scope': 'Native ARM64 real dependency resolution and original libSystem initializer probes for each bundle. App-main, rendering and gameplay untested.',
              'source_hashes': baseline, 'test_binary': executable,
              'test_binary_sha256': file_digest(executable),
              'build_source_verified': bool(args.build_proof),
              'build_proof': str(args.build_proof.resolve()) if args.build_proof else None,
              'source_snapshot_scope': 'All native src files, tests/a64 fixtures, Cargo.toml/Cargo.lock/build.rs; not external toolchain/vendor dependencies',
              'cache': str(args.cache), 'cache_main_size': args.cache.stat().st_size,
              'cache_main_mtime_ns': args.cache.stat().st_mtime_ns,
              'started_utc': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()), 'results': []}
    if not args.build_proof:
        print('WARNING: no build proof; source stability does not verify binary/source association', flush=True)
    report['probe'] = args.probe
    if args.probe == 'legacy-cpp-prepare':
        report['scope'] = 'Native original iOS11 cache legacy C++ dependency/binding diagnostic. Initializers, app-main and gameplay untested.'
    inventory = json.loads(args.inventory.read_text())
    records = inventory['ipas']
    for index, ipa in enumerate(records):
        result = dict(ipa, app_gameplay_verified=False)
        if ipa.get('duplicate_of'):
            result['status'] = 'duplicate'
        elif ipa.get('error'):
            result['status'] = 'inventory_error'
        elif not ipa.get('has_arm64'):
            result['status'] = 'not_arm64'
        elif any(s.get('encrypted') for s in ipa['slices'] if s['architecture'] == 'arm64'):
            result['status'] = 'encrypted_arm64_skipped'
        else:
            if hashes(args.native) != baseline:
                raise SystemExit('Frozen source changed; refusing to mix builds in regression')
            if file_digest(executable) != report['test_binary_sha256']:
                raise SystemExit('Frozen test binary changed; refusing to mix artifacts')
            env = dict(os.environ, PLAYCOVER_NATIVE_IPA=ipa['path'], PLAYCOVER_NATIVE_CACHE=str(args.cache), SDL_VIDEODRIVER='dummy', SDL_AUDIODRIVER='dummy')
            env['PLAYCOVER_REGRESSION_PROBE'] = args.probe
            start = time.monotonic()
            command = [executable, 'a64::native_initializer_tests::actual_ipa_regression', '--exact', '--ignored', '--nocapture', '--test-threads=1']
            try:
                proc = subprocess.run(command, cwd=args.native, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=args.timeout)
                output, code = proc.stdout, proc.returncode
            except subprocess.TimeoutExpired as error:
                output = error.stdout or b''
                if isinstance(output, bytes):
                    output = output.decode(errors='replace')
                code = None
            result.update(returncode=code, elapsed_seconds=round(time.monotonic() - start, 3))
            for key, marker in [('boundary', 'BOUNDARY'), ('stage', 'STAGE'), ('mode', 'MODE')]:
                result[key] = next((line.split(f'PLAYCOVER_REGRESSION_{marker}: ', 1)[1] for line in output.splitlines() if f'PLAYCOVER_REGRESSION_{marker}: ' in line), None)
            collected = 'PLAYCOVER_REGRESSION_COLLECTED:' in output
            result['status'] = 'timeout' if code is None else 'harness_error' if code != 0 or not collected else 'blocked' if result['boundary'] else 'diagnostic_completed'
            log = f'{index:03d}-{ipa["sha256"][:12]}.log'
            (args.output / log).write_text(output)
            result['log'] = log
            print(f'{index+1}/{len(records)} {ipa.get("name", Path(ipa["path"]).name)}: {result["status"]} {result.get("boundary") or result.get("stage") or ""}', flush=True)
        report['results'].append(result)
        (args.output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    report['frozen_source_verified_after_run'] = hashes(args.native) == baseline
    report['frozen_binary_verified_after_run'] = file_digest(executable) == report['test_binary_sha256']
    report['finished_utc'] = time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())
    groups = defaultdict(list)
    for item in report['results']:
        if item.get('boundary'):
            groups[fingerprint(item['boundary'])].append(item)
    report['common_blockers'] = [{'boundary': key, 'count': len(items), 'apps': [i.get('name') for i in items]} for key, items in sorted(groups.items(), key=lambda pair: -len(pair[1]))]
    (args.output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    escape = lambda s: str(s or '').replace('|', '\\|').replace('\n', ' ')
    text = ['# ARM64 IPA regression', '', report['scope'], '', 'This inventories all local ARM64 backups, including non-games. Each unique unencrypted backup is tested once. A first failure can hide later issues; diagnostic completion does not mean an app boots.', '', '## Common first blockers', '', '| Apps | Boundary |', '| --- | --- |']
    text += [f'| {group["count"]} | {escape(group["boundary"])} |' for group in report['common_blockers']]
    text += ['', '## Per IPA', '', '| App | Version / iOS minimum | Status | Probe / first boundary |', '| --- | --- | --- | --- |']
    text += [f'| {escape(i.get("name", Path(i["path"]).name))} | {escape(i.get("version"))} / {escape(i.get("minimum_ios"))} | {i["status"]} | {escape(i.get("boundary") or i.get("stage") or i.get("error") or i.get("duplicate_of"))} |' for i in report['results']]
    (args.output / 'REPORT.md').write_text('\n'.join(text) + '\n')
    print(json.dumps({'tested': sum(i['status'] in ('blocked', 'diagnostic_completed', 'timeout', 'harness_error') for i in report['results']), 'groups': len(groups), 'source_stable': report['frozen_source_verified_after_run']}))
    if not report['frozen_source_verified_after_run'] or not report['frozen_binary_verified_after_run']:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
