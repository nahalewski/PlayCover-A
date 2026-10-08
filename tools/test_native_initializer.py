#!/usr/bin/env python3
"""Run in WSL: opt-in actual Terraria CPU probe without Android packaging.

Root is the sole builder. Register/copy a64_native_initializer_tests.rs first.
This runner refuses a stale source snapshot and records the actual outcome.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import time


def digest(path):
    result = hashlib.sha256()
    with path.open('rb') as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b''):
            result.update(chunk)
    return result.hexdigest()


def snapshot(project, native):
    pending = [Path('src/a64.rs'), Path('src/lib.rs')]
    paths = {Path('src/cpu/dynarmic_wrapper/a64.rs'), Path('src/cpu/dynarmic_wrapper/a64.cpp')}
    while pending:
        relative = pending.pop()
        if relative in paths:
            continue
        paths.add(relative)
        text = (project / relative).read_text()
        for name in re.findall(r'#\[path\s*=\s*"([^"]+)"\]', text):
            # Registered A64 source graph, including cfg(test) native harness.
            if name.startswith('a64_') and name.endswith('.rs'):
                pending.append(relative.parent / name)
    required = Path('src/a64_native_initializer_tests.rs')
    if required not in paths:
        raise ValueError('Root must register the native test module in a64.rs')
    hashes = {}
    for relative in sorted(paths):
        frozen = digest(project / relative)
        if not (native / relative).is_file() or digest(native / relative) != frozen:
            raise ValueError(f'Native snapshot is stale: {relative}; root must copy frozen source first')
        hashes[str(relative)] = frozen
    return hashes


def main():
    workspace = Path(__file__).resolve().parent.parent
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--ipa', type=Path, default=Path('/mnt/c/Users/Ben/Desktop/ipa/Terraria_4.5.0.ipa'))
    parser.add_argument('--cache', type=Path, default=workspace / 'ios-runtime/cache/System/Library/Caches/com.apple.dyld/dyld_shared_cache_arm64')
    parser.add_argument('--native', type=Path, default=Path('/home/ben/touchHLE-a64-integration'))
    parser.add_argument('--target', type=Path, default=Path('/home/ben/touchHLE-a64/target'))
    parser.add_argument('--label', required=True)
    parser.add_argument('--expect-boundary')
    parser.add_argument('--timeout', type=int, default=180)
    parser.add_argument('--verbose', action='store_true', help='Print the complete cargo output; full log is always saved')
    args = parser.parse_args()
    if not re.fullmatch(r'[a-z0-9-]+', args.label):
        parser.error('label must be lowercase letters, digits and hyphens')
    args.ipa = args.ipa.resolve(strict=True)
    args.cache = args.cache.resolve(strict=True)
    hashes = snapshot(workspace / 'touchHLE-src', args.native)
    env = dict(os.environ, CARGO_TARGET_DIR=str(args.target),
               PLAYCOVER_NATIVE_IPA=str(args.ipa), PLAYCOVER_NATIVE_CACHE=str(args.cache),
               SDL_VIDEODRIVER='dummy', SDL_AUDIODRIVER='dummy')
    env.pop('PLAYCOVER_NATIVE_EXPECT_BOUNDARY', None)
    if args.expect_boundary is not None:
        if not args.expect_boundary:
            parser.error('expected boundary cannot be empty')
        env['PLAYCOVER_NATIVE_EXPECT_BOUNDARY'] = args.expect_boundary
    cargo = Path.home() / '.cargo/bin/cargo'
    command = [str(cargo), 'test', '--lib', '--features', 'a64',
               'a64::native_initializer_tests::actual_terraria_original_libsystem_initializer',
               '--', '--exact', '--ignored', '--nocapture', '--test-threads=1']
    report = {'scope': 'Native x86 host running same real guest A64 diagnostic; no Android/device milestone.',
              'ipa': str(args.ipa), 'ipa_sha256': digest(args.ipa),
              'cache': str(args.cache), 'cache_main_sha256': digest(args.cache),
              'source_hashes': hashes, 'command': command, 'expected_boundary': args.expect_boundary,
              'app_gameplay_verified': False, 'runtime_readiness_receipt': False}
    start = time.monotonic()
    try:
        process = subprocess.run(command, cwd=args.native, env=env, stdout=subprocess.PIPE,
                                 stderr=subprocess.STDOUT, text=True, timeout=args.timeout)
        output, code = process.stdout, process.returncode
    except subprocess.TimeoutExpired as error:
        output = error.stdout or b''
        if isinstance(output, bytes):
            output = output.decode(errors='replace')
        output += '\nNative probe timed out; no success inferred.\n'
        code = None
    report.update(elapsed_seconds=round(time.monotonic() - start, 3), returncode=code,
                  boundary=next((line.split('PLAYCOVER_NATIVE_BOUNDARY: ', 1)[1] for line in output.splitlines() if 'PLAYCOVER_NATIVE_BOUNDARY: ' in line), None),
                  outcome=next((line.split('PLAYCOVER_NATIVE_OUTCOME: ', 1)[1] for line in output.splitlines() if 'PLAYCOVER_NATIVE_OUTCOME: ' in line), None))
    try:
        final_hashes = snapshot(workspace / 'touchHLE-src', args.native)
        stable = final_hashes == hashes
        stability_error = None if stable else 'Registered source hashes changed during native run'
    except (ValueError, OSError) as error:
        stable, stability_error = False, str(error)
    report.update(frozen_source_verified_after_run=stable, source_stability_error=stability_error)
    destination = workspace / 'pixel-fold-tests'
    destination.mkdir(exist_ok=True)
    prefix = destination / f'terraria-native-{args.label}'
    prefix.with_suffix('.log').write_text(output)
    prefix.with_suffix('.json').write_text(json.dumps(report, indent=2) + '\n')
    if args.verbose:
        print(output, end='')
    else:
        for line in output.splitlines():
            if '[a64]' in line or 'PLAYCOVER_NATIVE_' in line:
                print(line)
    print(json.dumps({key: report[key] for key in ('elapsed_seconds', 'returncode', 'boundary', 'outcome',
                                                  'frozen_source_verified_after_run', 'source_stability_error')}, indent=2))
    if code != 0 or not report['outcome'] or not stable:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
