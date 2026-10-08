"""Fetch reference source only; never build it or modify the emulator tree."""
import concurrent.futures
import datetime
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1] / 'runtime-sources'
SOURCES = [
    ('apple-libdispatch', 'apple-oss-distributions/libdispatch', 'dispatch queues and once resolvers'),
    ('apple-libpthread', 'apple-oss-distributions/libpthread', 'Darwin threads and synchronization'),
    ('apple-libplatform', 'apple-oss-distributions/libplatform', 'platform primitives and atomic implementations'),
    ('apple-objc4', 'apple-oss-distributions/objc4', 'Objective-C runtime'),
    ('apple-dyld', 'apple-oss-distributions/dyld', 'Mach-O loading and runtime initialization'),
    ('apple-xnu', 'apple-oss-distributions/xnu', 'Mach and Darwin kernel reference'),
    ('swift', 'swiftlang/swift', 'Swift language and runtime source'),
    ('darling', 'darlinghq/darling', 'Darwin compatibility implementation for Linux'),
    ('darling-foundation', 'darlinghq/darling-foundation', 'Foundation reimplementation'),
    ('darling-corefoundation', 'darlinghq/darling-corefoundation', 'CoreFoundation reimplementation'),
    ('indium', 'darlinghq/indium', 'Metal-like API and shader translation over Vulkan'),
    ('darlingserver', 'darlinghq/darlingserver', 'Linux userspace Mach and Darwin service server'),
    ('darling-libdispatch', 'darlinghq/darling-libdispatch', 'Darling adaptations of dispatch'),
    ('darling-libpthread', 'darlinghq/darling-libpthread', 'Darling adaptations of Darwin pthreads'),
    ('darling-libplatform', 'darlinghq/darling-libplatform', 'Darling platform and atomic adaptations'),
    ('darling-objc4', 'darlinghq/darling-objc4', 'Darling Objective-C runtime adaptations'),
    ('darling-dyld', 'darlinghq/darling-dyld', 'Darling dynamic loader adaptations'),
    ('darling-metal', 'darlinghq/darling-metal', 'Darling Objective-C Metal frontend'),
]

def git(*args, timeout=300):
    result = subprocess.run(['git', *args], capture_output=True, text=True,
                            encoding='utf-8', errors='replace', timeout=timeout)
    if result.returncode:
        raise RuntimeError(result.stderr.strip()[-2000:])
    return result.stdout.strip()

def fetch(source):
    name, repo, purpose = source
    url = f'https://github.com/{repo}'
    path = ROOT / name
    item = dict(name=name, url=url, purpose=purpose, path=str(path), downloaded=False,
                built=False, integrated=False)
    try:
        if path.exists():
            if not (path / '.git').is_dir():
                raise RuntimeError('Existing non-repository directory; left untouched')
            actual = git('-C', str(path), 'remote', 'get-url', 'origin')
            if actual.removesuffix('.git') != url:
                raise RuntimeError('Existing origin differs; left untouched')
        else:
            git('-c', 'core.longpaths=true', 'clone', '--depth', '1', '--single-branch', url + '.git', str(path))
        item['commit'] = git('-C', str(path), 'rev-parse', 'HEAD')
        item['commit_time'] = git('-C', str(path), 'show', '-s', '--format=%cI', 'HEAD')
        item['license_files'] = [str(p.relative_to(path)) for p in path.iterdir()
                                 if p.is_file() and any(word in p.name.upper()
                                 for word in ['LICENSE', 'COPYING', 'COPYRIGHT'])]
        item['submodules'] = [line for line in git('-C', str(path), 'ls-files', '--stage').splitlines()
                              if line.startswith('160000 ')]
        item['submodules_fetched'] = False
        item['downloaded'] = True
    except Exception as error:
        item['error'] = str(error)
    print(f"{name}: {'downloaded ' + item.get('commit', '')[:12] if item['downloaded'] else item['error']}", flush=True)
    return item

if __name__ == '__main__':
    ROOT.mkdir(exist_ok=True)
    with concurrent.futures.ThreadPoolExecutor(max_workers=3) as pool:
        items = list(pool.map(fetch, SOURCES))
    report = dict(fetched_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),
                  scope='Reference sources, shallow checkouts. No builds, integration, or submodule execution.',
                  sources=items,
                  remaining=['Android ARM64 build and guest/host ABI integration',
                             'Darwin syscall and Mach IPC implementation',
                             'Apple cache resolvers, commpage, and runtime initialization',
                             'ARM64 Objective-C class registration and Swift runtime compatibility',
                             'Guest threading, TLS, synchronization and scheduler integration',
                             'Unity dynamic loading and initializer ordering',
                             'iOS UIKit, window/input/audio/device-service integration',
                             'ABI-compatible Metal layer, shader support and Android Vulkan presentation',
                             'Exact revision/dependency selection and applicable notices before reuse',
                             'Real-app execution and gameplay verification'])
    (ROOT / 'manifest.json').write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
    if not all(item['downloaded'] for item in items):
        raise SystemExit(1)
