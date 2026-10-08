"""Verify that all currently imported first-generation Fold IPAs display icons."""
import hashlib
import json
from pathlib import Path
import subprocess
import time
import xml.etree.ElementTree as ET
from adb_device import discover_device

serial = discover_device('felix')
package = 'org.touchhle.android.a64test'
folder = Path('pixel-fold-tests')
folder.mkdir(exist_ok=True)

def adb(*args):
    return subprocess.check_output(['adb', '-s', serial, *args], timeout=60,
                                   text=True, encoding='utf-8', errors='replace')

adb('shell', 'am', 'force-stop', package)
adb('shell', 'am', 'start', '-n', package + '/org.touchhle.android.LauncherActivity')
expected = ['Angry Birds', 'Pocket God', 'Sonic 1', 'Sonic 2']
for attempt in range(12):
    time.sleep(1)
    focus = adb('shell', 'dumpsys', 'window')
    assert any(package in line for line in focus.splitlines() if 'mCurrentFocus=' in line)
    adb('shell', 'uiautomator', 'dump', '/data/local/tmp/imported-icons.xml')
    xml = adb('shell', 'cat', '/data/local/tmp/imported-icons.xml')
    root = ET.fromstring(xml)
    icons = [n.attrib.get('content-desc', '') for n in root.iter('node')
             if n.attrib.get('class') == 'android.widget.ImageView'
             and n.attrib.get('content-desc', '').endswith(' icon')]
    if all(any(name.lower() in icon.lower() for icon in icons) for name in expected):
        break
report = {'serial': serial, 'icons': icons,
          'passed': all(any(name.lower() in icon.lower() for icon in icons) for name in expected),
          'apk_sha256': hashlib.sha256(Path('PlayCover-A-9Pro-test.apk').read_bytes()).hexdigest()}
(folder / 'imported-icons-gen1.xml').write_text(xml, encoding='utf-8')
adb('shell', 'screencap', '-p', '/data/local/tmp/imported-icons.png')
adb('pull', '/data/local/tmp/imported-icons.png', str(folder / 'imported-icons-gen1.png'))
(folder / 'imported-icons-gen1.json').write_text(json.dumps(report, indent=2), encoding='utf-8')
print(json.dumps(report))
assert report['passed'], 'Some imported apps still display placeholders'
