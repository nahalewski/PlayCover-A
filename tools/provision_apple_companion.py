"""Provision local companion tokens without printing them."""
from pathlib import Path
import json
import subprocess

token = (Path.home()/'.playcover-a'/'companion-token').read_text().strip()
assert len(token) >= 32
data = json.dumps({'url':'http://127.0.0.1:18765','token':token}).encode()
for serial in ('R52Y8066STA','192.168.0.22:41105'):
    cmd = ['adb','-P','5038','-s',serial]
    subprocess.run(cmd+['reverse','tcp:18765','tcp:18765'],check=True,capture_output=True)
    subprocess.run(cmd+['shell','run-as','org.touchhle.android.a64test','mkdir','-p','no_backup'],check=True)
    subprocess.run(cmd+['shell','run-as','org.touchhle.android.a64test','sh','-c',
        '"umask 077; cat > no_backup/apple-store-companion.json"'],input=data,check=True,capture_output=True)
    print(serial+': local companion connection provisioned')
