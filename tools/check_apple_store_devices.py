"""Verify App Store navigation on the two authorized Android devices."""
import subprocess
import xml.etree.ElementTree as ET
import re
from pathlib import Path

root = Path('pixel-fold-tests')
root.mkdir(exist_ok=True)
for serial, label in [('R52Y8066STA', 'apple-store-tab'), ('192.168.0.22:41105', 'apple-store-fold9')]:
    def adb(*args):
        return subprocess.check_output(['adb', '-P', '5038', '-s', serial, *args], timeout=120)
    adb('shell', 'am', 'force-stop', 'org.touchhle.android.a64test')
    adb('shell', 'am', 'start', '-n', 'org.touchhle.android.a64test/org.touchhle.android.LauncherActivity')
    adb('shell', 'uiautomator', 'dump', '/sdcard/apple-store-ui.xml')
    tree = ET.fromstring(adb('shell', 'cat', '/sdcard/apple-store-ui.xml'))
    node = next(n for n in tree.iter('node') if n.get('text') == 'App Store')
    x1, y1, x2, y2 = map(int, re.findall(r'\d+', node.get('bounds')))
    adb('shell', 'input', 'tap', str((x1+x2)//2), str((y1+y2)//2))
    adb('shell', 'uiautomator', 'dump', '/sdcard/apple-store-ui.xml')
    data = adb('shell', 'cat', '/sdcard/apple-store-ui.xml')
    assert b'Apple account downloads' in data, data
    (root / (label+'.xml')).write_bytes(data)
    (root / (label+'.png')).write_bytes(adb('exec-out', 'screencap', '-p'))
    print(label + ': App Store page verified', flush=True)
    def tap_text(text):
        adb('shell', 'uiautomator', 'dump', '/sdcard/apple-store-ui.xml')
        tree = ET.fromstring(adb('shell', 'cat', '/sdcard/apple-store-ui.xml'))
        node = next(n for n in tree.iter('node') if n.get('text') == text)
        x1,y1,x2,y2 = map(int,re.findall(r'\d+',node.get('bounds')))
        adb('shell','input','tap',str((x1+x2)//2),str((y1+y2)//2))
    tap_text('Apple account downloads')
    tap_text('Check sign-in status')
    adb('shell','uiautomator','dump','/sdcard/apple-store-ui.xml')
    data=adb('shell','cat','/sdcard/apple-store-ui.xml')
    assert b'Companion connected' in data, 'Companion status not verified'
    (root/(label+'-account.xml')).write_bytes(data)
    (root/(label+'-account.png')).write_bytes(adb('exec-out','screencap','-p'))
    print(label+': companion connected, sign-in required',flush=True)
