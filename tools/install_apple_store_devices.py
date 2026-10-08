"""Install the same verified APK on the two requested devices, preserving data."""
from pathlib import Path
import subprocess
import hashlib
import json

apk = Path('PlayCover-A-9Pro-test.apk')
digest = hashlib.file_digest(apk.open('rb'),'sha256').hexdigest()
rows = []
for serial in ('R52Y8066STA','192.168.0.22:41105'):
    prefix = ['adb','-P','5038','-s',serial]
    subprocess.run(prefix+['install','-r',str(apk)],check=True,timeout=600)
    path = subprocess.check_output(prefix+['shell','pm','path','org.touchhle.android.a64test'],text=True).strip().removeprefix('package:')
    actual = subprocess.check_output(prefix+['shell','sha256sum',path],text=True).split()[0]
    assert actual == digest
    rows.append(dict(serial=serial,apk_sha256=actual,installed=True))
    print(serial+': installed APK hash verified',flush=True)
Path('pixel-fold-tests/apple-store-installed.json').write_text(json.dumps(rows,indent=2))
