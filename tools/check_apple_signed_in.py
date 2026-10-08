import subprocess
import xml.etree.ElementTree as ET
import re
from pathlib import Path
for serial,label in [('R52Y8066STA','tab'),('192.168.0.22:41105','fold9')]:
    def adb(*args):
        return subprocess.check_output(['adb','-P','5038','-s',serial,*args],timeout=60)
    adb('reverse','tcp:18765','tcp:18765')
    adb('shell','uiautomator','dump','/sdcard/apple-store-ui.xml')
    tree=ET.fromstring(adb('shell','cat','/sdcard/apple-store-ui.xml'))
    node=next((n for n in tree.iter('node') if n.get('text')=='Check sign-in status'),None)
    if node is None:
        print(label+': companion connected; user can refresh account page')
        continue
    x1,y1,x2,y2=map(int,re.findall(r'\d+',node.get('bounds')))
    adb('shell','input','tap',str((x1+x2)//2),str((y1+y2)//2))
    adb('shell','uiautomator','dump','/sdcard/apple-store-ui.xml')
    data=adb('shell','cat','/sdcard/apple-store-ui.xml')
    assert b'Signed in on the computer' in data
    Path('pixel-fold-tests/apple-signed-in-'+label+'.xml').write_bytes(data)
    print(label+': Signed in on the computer verified')
