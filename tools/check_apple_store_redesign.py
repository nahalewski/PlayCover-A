import subprocess
import xml.etree.ElementTree as ET
import re
from pathlib import Path
import sys
import time

for serial,label in [('R52Y8066STA','tab'),('192.168.0.22:41105','fold9')]:
    if len(sys.argv)>1 and label!=sys.argv[1]: continue
    def adb(*args):
        return subprocess.check_output(['adb','-P','5038','-s',serial,*args],timeout=120)
    def screenshot():
        data=adb('exec-out','screencap','-p')
        start=data.find(b'\x89PNG\r\n\x1a\n')
        assert start>=0
        return data[start:]
    adb('shell','am','force-stop','org.touchhle.android.a64test')
    adb('shell','am','start','-n','org.touchhle.android.a64test/org.touchhle.android.LauncherActivity')
    adb('shell','uiautomator','dump','/sdcard/apple-store-ui.xml')
    nodes=ET.fromstring(adb('shell','cat','/sdcard/apple-store-ui.xml'))
    node=next(n for n in nodes.iter('node') if n.get('text')=='App Store')
    x1,y1,x2,y2=map(int,re.findall(r'\d+',node.get('bounds')))
    adb('shell','input','tap',str((x1+x2)//2),str((y1+y2)//2))
    for attempt in range(15):
        adb('shell','uiautomator','dump','/sdcard/apple-store-ui.xml')
        data=adb('shell','cat','/sdcard/apple-store-ui.xml')
        if b'50 of 1993 purchased apps' in data: break
        time.sleep(2)
    assert b'50 of 1993 purchased apps' in data, 'Initial purchases did not load'
    Path('pixel-fold-tests/apple-store-redesign-'+label+'.xml').write_bytes(adb('shell','cat','/sdcard/apple-store-ui.xml'))
    Path('pixel-fold-tests/apple-store-redesign-'+label+'.png').write_bytes(screenshot())
    print(label+': redesigned App Store opened')
    nodes=ET.fromstring(data)
    assert not any(n.get('text')=='Companion URL' for n in nodes.iter('node'))
    node=next(n for n in nodes.iter('node') if n.get('text')=='Load more purchases')
    x1,y1,x2,y2=map(int,re.findall(r'\d+',node.get('bounds')))
    adb('shell','input','tap',str((x1+x2)//2),str((y1+y2)//2))
    for attempt in range(15):
        adb('shell','uiautomator','dump','/sdcard/apple-store-ui.xml')
        data=adb('shell','cat','/sdcard/apple-store-ui.xml')
        if b'100 of 1993 purchased apps' in data: break
        time.sleep(2)
    assert b'100 of 1993 purchased apps' in data, 'Second purchase page did not append'
    Path('pixel-fold-tests/apple-store-redesign-'+label+'-100.xml').write_bytes(data)
    Path('pixel-fold-tests/apple-store-redesign-'+label+'-100.png').write_bytes(screenshot())
    print(label+': 100/1993 purchases appended; connection controls absent from browse')
