import subprocess,re
import xml.etree.ElementTree as ET
from pathlib import Path
for serial,label in [('R52Y8066STA','tab'),('192.168.0.22:41105','fold9')]:
    def adb(*args): return subprocess.check_output(['adb','-P','5038','-s',serial,*args],timeout=90)
    def snapshot():
        adb('shell','uiautomator','dump','/sdcard/apple-store-ui.xml')
        return ET.fromstring(adb('shell','cat','/sdcard/apple-store-ui.xml'))
    def tap(node):
        x1,y1,x2,y2=map(int,re.findall(r'\d+',node.get('bounds')))
        adb('shell','input','tap',str((x1+x2)//2),str((y1+y2)//2))
    def text(value): return next(n for n in snapshot().iter('node') if n.get('text')==value)
    def image(suffix):
        data=adb('exec-out','screencap','-p');start=data.index(b'\x89PNG\r\n\x1a\n')
        Path('pixel-fold-tests/store-'+label+'-'+suffix+'.png').write_bytes(data[start:])
    tree=snapshot(); listing=next(n for n in tree.iter('node') if n.get('class')=='android.widget.ListView')
    tap(list(listing)[0]);tree=snapshot()
    assert any('Current store version' in n.get('text','') for n in tree.iter('node'))
    image('details');tap(text('Close'));tap(text('Back to library'));tap(text('Settings'))
    tap(text('Apple account and connection'));tree=snapshot()
    assert any(n.get('text')=='App Store account' for n in tree.iter('node'))
    assert any(n.get('text')=='Save connection' for n in tree.iter('node'))
    # Do not screenshot connection settings containing the bearer-token field.
    tap(text('Back to settings'))
    print(label+': detail view and Settings account route verified')
