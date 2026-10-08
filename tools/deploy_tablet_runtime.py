"""Stage unchanged original ARM64 cache on the authorized tablet, SHA verified."""
from pathlib import Path
import subprocess,hashlib,json,shlex
source=Path('ios-runtime/cache/System/Library/Caches/com.apple.dyld')
remote='/sdcard/Android/data/org.touchhle.android.a64test/files/ios-runtime/cache'
prefix=['adb','-P','5038','-s','R52Y8066STA']
def adb(*args):return subprocess.check_output(prefix+list(args),timeout=300,text=True)
assert adb('shell','getprop','ro.product.device').strip()=='gts11uwifi'
print(adb('shell','df','-h','/sdcard'),flush=True)
adb('shell','mkdir','-p',remote)
files=sorted(source.glob('dyld_shared_cache_arm64*'))
assert len(files)==44
records=[]
for p in files:
    with p.open('rb') as stream:digest=hashlib.file_digest(stream,'sha256').hexdigest()
    dest=remote+'/'+p.name
    result=subprocess.run(prefix+['shell','sha256sum',dest],capture_output=True,text=True)
    if result.returncode!=0:
        adb('push',str(p),dest+'.part')
        assert adb('shell','sha256sum',dest+'.part').split()[0]==digest
        adb('shell','mv',dest+'.part',dest)
    else:assert result.stdout.split()[0]==digest,'Different existing runtime preserved'
    records.append(dict(file=p.name,bytes=p.stat().st_size,sha256=digest))
    print(f'{len(records)}/{len(files)} verified: {p.name}',flush=True)
Path('pixel-fold-tests/tablet-ios16-runtime.json').write_text(json.dumps(dict(remote=remote,files=records,execution_verified=False),indent=2))
