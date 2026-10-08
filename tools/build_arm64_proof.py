#!/usr/bin/env python3
"""Builder-owned Cargo build with pre/post source hashes and artifact receipt."""
import argparse
import json
import os
from pathlib import Path
import subprocess

from regress_arm64 import hashes, file_digest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native', type=Path, required=True)
    parser.add_argument('--build-json', type=Path, required=True)
    parser.add_argument('--proof', type=Path, required=True)
    parser.add_argument('--target', type=Path)
    parser.add_argument('--cargo', default=str(Path.home() / '.cargo/bin/cargo'))
    args = parser.parse_args()
    if args.proof.resolve() == args.build_json.resolve():
        parser.error('Proof and Cargo JSON must be separate files')
    before = hashes(args.native)
    environment = dict(os.environ)
    if args.target:
        environment['CARGO_TARGET_DIR'] = str(args.target.resolve())
    command = [args.cargo, 'test', '--lib', '--features', 'a64', '--no-run', '--message-format=json']
    with args.build_json.open('w', encoding='utf-8') as output:
        result = subprocess.run(command, cwd=args.native, env=environment, stdout=output)
    after = hashes(args.native)
    if result.returncode or before != after:
        raise SystemExit('Build failed or source changed during build; no valid proof issued')
    artifacts, finished = set(), False
    for line in args.build_json.read_text().splitlines():
        try:
            item = json.loads(line)
        except ValueError:
            continue
        if item.get('reason') == 'build-finished':
            finished = item.get('success') is True
        if (item.get('reason') == 'compiler-artifact' and item.get('profile', {}).get('test')
                and item.get('executable') and set(item.get('target', {}).get('kind', [])) & {'lib', 'rlib', 'cdylib'}):
            artifacts.add(item['executable'])
    if not finished or len(artifacts) != 1:
        raise SystemExit('Cargo did not provide one successful library test artifact')
    executable = next(iter(artifacts))
    proof = {'schema': 1, 'build_succeeded': True, 'source_hashes_before': before,
             'source_hashes_after': after, 'test_binary': executable,
             'test_binary_sha256': file_digest(executable),
             'build_json_sha256': file_digest(args.build_json), 'command': command,
             'native': str(args.native.resolve()),
             'scope': 'Native src, tests/a64 and Cargo inputs; no external toolchain/vendor attestation'}
    temporary = args.proof.with_name(args.proof.name + '.tmp')
    temporary.write_text(json.dumps(proof, indent=2) + '\n')
    temporary.replace(args.proof)
    print(json.dumps({'proof': str(args.proof), 'binary': executable}))


if __name__ == '__main__':
    main()
