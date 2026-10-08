#!/usr/bin/env python3
"""Run only the original-cache audited capability resolver; no app startup."""
import argparse, datetime, hashlib, json, subprocess, time, uuid
from pathlib import Path

workspace = Path(__file__).resolve().parent.parent
p=argparse.ArgumentParser(description=__doc__)
p.add_argument('--binary',type=Path,default=Path('/home/ben/touchHLE-a64/target/debug/touchHLE'))
p.add_argument('--cache',type=Path,default=workspace/'ios-runtime/cache/System/Library/Caches/com.apple.dyld/dyld_shared_cache_arm64')
p.add_argument('--output',type=Path,default=workspace/'pixel-fold-tests/ios16-audited-dispatch-resolver-desktop.json')
args=p.parse_args()
def sha(path):
    h=hashlib.sha256()
    with path.open('rb') as f:
        for b in iter(lambda:f.read(1048576),b''): h.update(b)
    return h.hexdigest()
with args.cache.open('rb') as f:
    header=f.read(104)
cache_uuid=str(uuid.UUID(bytes=header[88:104]))
assert cache_uuid=='32035564-853b-388b-a1ef-2ea354c196e1',cache_uuid
command=[str(args.binary),f'--a64-cache-resolver-test={args.cache}']
start=time.monotonic()
result=subprocess.run(command,cwd=args.binary.parent.parent.parent,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True,timeout=180)
args.output.parent.mkdir(parents=True,exist_ok=True)
log=args.output.with_suffix('.log');log.write_text(result.stdout)
report={'tested_at':datetime.datetime.now(datetime.timezone.utc).isoformat(),'command':command,
        'binary_sha256':sha(args.binary),'cache_main_sha256':sha(args.cache),'cache_uuid':cache_uuid,
        'exit_code':result.returncode,'elapsed_seconds':round(time.monotonic()-start,3),'stdout_file':str(log),
        'resolver_probe_passed':result.returncode==0 and 'resolver returned 0x18db355c0' in result.stdout,
        'resolved_implementation_executed':False,'app_initializers_executed':False,'app_main_executed':False,
        'gameplay_verified':False,'scope':'Exact byte-audited iOS16 dispatch_once_f CPU-capability selector only; no application supplied.'}
args.output.write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps(report,indent=2))
print(result.stdout)
raise SystemExit(0 if report['resolver_probe_passed'] else 1)
