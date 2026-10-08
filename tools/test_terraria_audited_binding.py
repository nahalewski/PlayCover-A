#!/usr/bin/env python3
"""Retest the actual Terraria binding graph with audited resolvers, never main."""
import datetime, hashlib, json, os, re, subprocess, time
from pathlib import Path
workspace=Path(__file__).resolve().parent.parent
binary=Path('/home/ben/touchHLE-a64/target/debug/touchHLE')
ipa=Path('/mnt/c/Users/Ben/Desktop/ipa/Terraria_4.5.0.ipa')
cache=workspace/'ios-runtime/cache/System/Library/Caches/com.apple.dyld/dyld_shared_cache_arm64'
output=workspace/'pixel-fold-tests/terraria-ios16-audited-prepare-desktop.json'
def sha(path):
    h=hashlib.sha256()
    with path.open('rb') as f:
        for b in iter(lambda:f.read(1048576),b''):h.update(b)
    return h.hexdigest()
ipa_sha=sha(ipa); binary_sha=sha(binary)
previous=json.loads((workspace/'pixel-fold-tests/terraria-ios16-prepare-desktop.json').read_text())
assert ipa_sha==previous['ipa_sha256'],'Actual IPA changed; do not silently reuse its previous audit'
probe=json.loads((workspace/'pixel-fold-tests/ios16-audited-dispatch-resolver-desktop.json').read_text())
assert binary_sha==probe['binary_sha256'],'Native binary changed after the genuine resolver probe'
command=[str(binary),str(ipa),'--no-error-popup',f'--a64-cache-prepare={cache}']
start=time.monotonic()
result=subprocess.run(command,cwd=workspace,env=dict(os.environ,SDL_VIDEODRIVER='dummy',SDL_AUDIODRIVER='dummy'),stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True,timeout=180)
log_path=output.with_suffix('.log');log_path.write_text(result.stdout)
blocker=next((line for line in result.stdout.splitlines() if line.startswith('Error:')),None)
summary=re.search(r'cache preparation passed: (\d+) ordinary images, (\d+) original-cache dependencies, (\d+) resolved import records \((\d+) against cache\)',result.stdout)
resolvers=re.search(r'audited capability resolvers executed: (\d+)',result.stdout)
report={'tested_at':datetime.datetime.now(datetime.timezone.utc).isoformat(),'command':command,
        'binary_sha256':binary_sha,'ipa_sha256':ipa_sha,'cache_uuid':probe['cache_uuid'],
        'cache_main_sha256':probe['cache_main_sha256'],'returncode':result.returncode,
        'elapsed_seconds':round(time.monotonic()-start,3),'first_blocker':blocker,'stdout_file':str(log_path),
        'binding_preparation_passed':result.returncode==0 and summary is not None,
        'ordinary_images':int(summary[1]) if summary else None,
        'cached_dependencies':int(summary[2]) if summary else None,
        'resolved_import_records':int(summary[3]) if summary else None,
        'cached_import_records':int(summary[4]) if summary else None,
        'audited_resolvers_executed':int(resolvers[1]) if resolvers else None,
        'app_initializers_executed':False,'app_main_executed':False,'gameplay_verified':False,
        'scope':'Actual unchanged Terraria IPA production cache-binding path; only exact audited capability resolver execution is permitted. Full Apple runtime initialization/main gate remains closed.'}
output.write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps(report,indent=2)); print(result.stdout)
