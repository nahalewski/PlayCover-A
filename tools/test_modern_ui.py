"""Check native settings persistence, repository filters and the IPA exit overlay."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shlex
import subprocess
import time
import xml.etree.ElementTree as ET

p = argparse.ArgumentParser()
p.add_argument('--serial', required=True)
p.add_argument('--ipa', required=True)
args = p.parse_args()
package = 'org.touchhle.android.a64test'
folder = Path('pixel-fold-tests')

def adb(*words):
    return subprocess.check_output(['adb', '-s', args.serial, *words], timeout=50,
                                   text=True, encoding='utf-8', errors='replace').strip()

def snapshot():
    focus = [x for x in adb('shell', 'dumpsys', 'window').splitlines() if 'mCurrentFocus=' in x]
    assert any(package in x for x in focus), focus
    adb('shell', 'uiautomator', 'dump', '/data/local/tmp/modern-ui.xml')
    return ET.fromstring(adb('shell', 'cat', '/data/local/tmp/modern-ui.xml'))

def node(root, text):
    return next(n for n in root.iter('node') if n.attrib.get('text') == text)

def tap(n):
    b = list(map(int, re.findall(r'\d+', n.attrib['bounds'])))
    adb('shell', 'input', 'tap', str((b[0]+b[2])//2), str((b[1]+b[3])//2))

def launch_library():
    adb('shell', 'am', 'force-stop', package)
    adb('shell', 'am', 'start', '-n', package + '/org.touchhle.android.LauncherActivity')
    time.sleep(2)

report = {'serial': args.serial, 'passed': False,
          'apk_sha256': hashlib.sha256(Path('PlayCover-A-fold-test.apk').read_bytes()).hexdigest()}
launch_library()
tap(node(snapshot(), 'Settings'))
root = snapshot()
for title in ['Internal resolution', 'Startup orientation']:
    assert any(title in n.attrib.get('text', '') for n in root.iter('node'))
choice = next(n for n in root.iter('node') if n.attrib.get('text', '').startswith('Internal resolution:'))
original_resolution = choice.attrib['text'].split('\n')[0].split(': ', 1)[1]
tap(choice)
tap(node(snapshot(), '2×'))
launch_library()
tap(node(snapshot(), 'Settings'))
root = snapshot()
choice = next(n for n in root.iter('node') if n.attrib.get('text', '').startswith('Internal resolution: 2×'))
report['settings_persisted'] = True
adb('shell', 'screencap', '-p', '/data/local/tmp/modern-settings.png')
adb('pull', '/data/local/tmp/modern-settings.png', str(folder / 'modern-settings-felix.png'))
tap(choice)
tap(node(snapshot(), original_resolution))
report['settings_restored_original'] = original_resolution
launch_library()
tap(node(snapshot(), 'Repositories'))
root = snapshot()
tap(next(n for n in root.iter('node') if 'CyPwn' in n.attrib.get('text', '')))
for _ in range(15):
    time.sleep(1)
    root = snapshot()
    if any(n.attrib.get('text') == 'Games' for n in root.iter('node')):
        break
for label in ['All', 'Games', 'Emulators', 'Other']:
    node(root, label)
tap(node(root, 'Games'))
root = snapshot()
report['games_filter_visible'] = True
adb('shell', 'screencap', '-p', '/data/local/tmp/repo-games.png')
adb('pull', '/data/local/tmp/repo-games.png', str(folder / 'repo-games-filter-felix.png'))
launch_library()
adb('shell', f'am start -n {package}/org.touchhle.android.MainActivity --es app_path {shlex.quote(args.ipa)}')
time.sleep(3)
root = snapshot()
exit_button = next(n for n in root.iter('node') if n.attrib.get('content-desc') == 'IPA menu')
adb('shell', 'screencap', '-p', '/data/local/tmp/ipa-exit-overlay.png')
adb('pull', '/data/local/tmp/ipa-exit-overlay.png', str(folder / 'ipa-exit-overlay-felix.png'))
tap(exit_button)
root = snapshot()
node(root, 'Match device rotation')
report['rotation_option_visible'] = True
tap(node(root, 'Exit IPA'))
time.sleep(3)
root = snapshot()
node(root, 'Installed IPAs')
result = subprocess.run(['adb', '-s', args.serial, 'shell', 'pidof', package + ':game'], capture_output=True)
assert not result.stdout.strip(), 'Game process did not exit'
report['overlay_exits_to_modern_library'] = True
report['game_process_exited'] = True
report['passed'] = True
(folder / 'modern-ui-felix.json').write_text(json.dumps(report, indent=2), encoding='utf-8')
print(json.dumps(report))
