"""Download Apple's non-PAC iPhone 8 firmware and verify its published hash."""
import hashlib
import concurrent.futures
import json
import pathlib
import time
import urllib.parse
import urllib.request

root = pathlib.Path(__file__).resolve().parents[1] / 'ios-runtime'
folder = root / 'firmware'
folder.mkdir(parents=True, exist_ok=True)
metadata_url = 'https://api.ipsw.me/v4/device/iPhone10,4?type=ipsw'
with urllib.request.urlopen(metadata_url, timeout=60) as response:
    metadata = json.load(response)
firmware = next(f for f in metadata['firmwares'] if f['version'].startswith('16.7.') and f.get('sha256sum'))
url = firmware['url']
parsed = urllib.parse.urlparse(url)
if parsed.scheme != 'https' or not parsed.hostname.endswith('.cdn-apple.com'):
    raise RuntimeError('Firmware URL is not an Apple CDN HTTPS URL')
name = pathlib.PurePosixPath(parsed.path).name
destination = folder / name
part = destination.with_suffix('.ipsw.part')
(folder / 'source.json').write_text(json.dumps({'metadata_url': metadata_url, **firmware}, indent=2), encoding='utf-8')
if not destination.exists():
    offset = part.stat().st_size if part.exists() else 0
    chunks = folder / 'download-chunks'
    chunks.mkdir(exist_ok=True)
    total = firmware['filesize']
    ranges = [(start, min(start + 256 * 1024 * 1024, total)) for start in range(offset, total, 256 * 1024 * 1024)]
    def fetch(bounds):
        start, end = bounds
        target = chunks / f'{start}-{end}.part'
        done = target.stat().st_size if target.exists() else 0
        if done > end - start:
            raise RuntimeError('Invalid partial chunk size')
        if done == end - start: return target
        request = urllib.request.Request(url, headers={'Range': f'bytes={start + done}-{end - 1}'})
        with urllib.request.urlopen(request, timeout=120) as response:
            if response.status != 206 or not response.headers.get('Content-Range', '').startswith(f'bytes {start + done}-{end - 1}/'):
                raise RuntimeError('Apple CDN did not return the requested range')
            with target.open('ab' if done else 'wb') as output:
                while chunk := response.read(4 * 1024 * 1024): output.write(chunk)
        if target.stat().st_size != end - start: raise RuntimeError('Incomplete firmware chunk')
        return target
    print(f"Downloading iOS {firmware['version']} ({total / 1e9:.2f} GB) from Apple using 8 connections", flush=True)
    with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
        futures = [pool.submit(fetch, bounds) for bounds in ranges]
        complete = offset
        for future in concurrent.futures.as_completed(futures):
            complete += future.result().stat().st_size
            print(f'Completed {complete / 1e9:.2f}/{total / 1e9:.2f} GB', flush=True)
    with part.open('ab') as output:
        for start, end in ranges:
            with (chunks / f'{start}-{end}.part').open('rb') as source:
                while chunk := source.read(8 * 1024 * 1024): output.write(chunk)
    if part.stat().st_size != firmware['filesize']:
        raise RuntimeError('Firmware size mismatch; partial file preserved')
    part.rename(destination)
print('Verifying firmware SHA-256', flush=True)
with destination.open('rb') as source:
    digest = hashlib.file_digest(source, 'sha256').hexdigest()
if digest != firmware['sha256sum']:
    raise RuntimeError('Firmware SHA-256 mismatch')
print(f'Verified: {destination}', flush=True)
