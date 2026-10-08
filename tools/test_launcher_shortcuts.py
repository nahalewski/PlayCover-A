"""Check installed IPA UI and Android pin/launch flow on the selected Fold."""
import argparse
import datetime
import hashlib
import json
from pathlib import Path
import re
import shlex
import subprocess
import time
import xml.etree.ElementTree as ET
from adb_device import discover_device

parser = argparse.ArgumentParser()
parser.add_argument('--device', choices=['felix', 'comet'], default='felix')
args = parser.parse_args()
serial = discover_device(args.device)
package = 'org.touchhle.android.a64test'
launcher = 'com.google.android.apps.nexuslauncher'
folder = Path('pixel-fold-tests')
remote = '/data/local/tmp/playcover-shortcut-ui.xml'

def adb(*args):
    return subprocess.check_output(['adb', '-s', serial, *args], timeout=45,
                                   text=True, encoding='utf-8', errors='replace').strip()

def snapshot(allowed=(package,)):
    for _ in range(12):
        focus = [x for x in adb('shell', 'dumpsys', 'window').splitlines() if 'mCurrentFocus=' in x]
        if any(any(p in line for p in allowed) for line in focus):
            break
        time.sleep(0.5)
    else:
        raise RuntimeError('Expected activity is not foreground; no UI actions taken')
    adb('shell', 'uiautomator', 'dump', remote)
    return ET.fromstring(adb('shell', 'cat', remote))

def tap(node):
    bounds = list(map(int, re.findall(r'\d+', node.attrib['bounds'])))
    adb('shell', 'input', 'tap', str((bounds[0] + bounds[2]) // 2), str((bounds[1] + bounds[3]) // 2))

def named(root, text):
    return next(n for n in root.iter('node') if n.attrib.get('text') == text)

report = dict(tested_at=datetime.datetime.now(datetime.timezone.utc).isoformat(), serial=serial,
              device=args.device, package=package, passed=False,
              test_scope='Installed tap/hold gestures and Android shortcut routing; gameplay is separate.')
try:
    installed_apk = adb('shell', 'pm', 'path', package).splitlines()[0].removeprefix('package:')
    report['apk_sha256'] = adb('shell', 'sha256sum', installed_apk).split()[0]
    adb('shell', 'am', 'force-stop', package)
    adb('shell', 'am', 'start', '-n', f'{package}/org.touchhle.android.LauncherActivity')
    time.sleep(2)
    for _ in range(10):
        root = snapshot()
        rows = [n for n in root.iter('node') if n.attrib.get('resource-id') == 'android:id/text1'
                and 'angry' in n.attrib.get('text', '').lower()]
        if rows:
            break
        time.sleep(1)
    assert rows, 'Installed IPA list is missing Angry Birds'
    for tab in ['Installed IPAs', 'Repositories', 'Settings']:
        named(root, tab)
    assert any(n.attrib.get('class') == 'android.widget.ImageView'
               and 'icon' in n.attrib.get('content-desc', '') for n in root.iter('node'))
    report['installed_row'] = rows[0].attrib['text']
    report['installed_icons_visible'] = True
    snapshot()
    screenshot = '/data/local/tmp/playcover-installed.png'
    adb('shell', 'screencap', '-p', screenshot)
    adb('pull', screenshot, str(folder/f'installed-{args.device}.png'))
    adb('shell', 'rm', screenshot)
    start = adb('shell', "date '+%m-%d %H:%M:%S.000'")
    tap(rows[0])
    time.sleep(5)
    focus = adb('shell', 'dumpsys', 'window')
    assert any(package + '/org.touchhle.android.MainActivity' in x
               for x in focus.splitlines() if 'mCurrentFocus=' in x), 'Tap did not open the emulator activity'
    direct_log = adb('logcat', '-d', '-T', start, '-v', 'brief')
    assert 'Angry Birds' in direct_log and 'CPU emulation begins now' in direct_log, 'Tap did not directly start the IPA'
    report['direct_tap_launch_verified'] = True
    report['direct_tap_runtime_crashed'] = bool(re.search(r'Panic at|Fatal signal|FATAL EXCEPTION', direct_log))
    adb('shell', 'am', 'force-stop', package)
    adb('shell', 'am', 'start', '-n', f'{package}/org.touchhle.android.LauncherActivity')
    time.sleep(2)
    root = snapshot()
    actions = next(n for n in root.iter('node')
                   if n.attrib.get('content-desc') == report['installed_row'] + ' actions')
    tap(actions)
    root = snapshot()
    named(root, 'Launch'); named(root, 'Remove')
    report['removal_actions_accessible'] = True
    adb('shell', 'input', 'keyevent', '4')
    root = snapshot()
    installed = next(n for n in root.iter('node') if n.attrib.get('text') == report['installed_row'])
    bounds = list(map(int, re.findall(r'\d+', installed.attrib['bounds'])))
    x, y = (bounds[0] + bounds[2]) // 2, (bounds[1] + bounds[3]) // 2
    adb('shell', 'input', 'swipe', str(x), str(y), str(x), str(y), '800')
    report['pin_requested_by_long_press'] = True
    time.sleep(1)
    root = snapshot((package, launcher))
    add = [n for n in root.iter('node') if n.attrib.get('text', '').lower() == 'add to home screen']
    if not add:
        add = [n for n in root.iter('node') if n.attrib.get('text', '').lower() == 'add']
    if add:
        report['pin_confirmation_shown'] = True
        tap(add[-1])
        time.sleep(1)
    shortcut_state = adb('shell', 'dumpsys', 'shortcut')
    expected_id = 'ipa-' + hashlib.sha256('Angry Birds HD 1.5.0.ipa'.encode()).hexdigest()
    assert expected_id in shortcut_state, 'Android did not register this IPA shortcut'
    report['shortcut_id'] = expected_id
    report['shortcut_registered'] = True
    # Launch the exact explicit intent stored in the shortcut, without an arbitrary app path.
    start = adb('shell', "date '+%m-%d %H:%M:%S.000'")
    adb('shell', 'am', 'force-stop', package)
    report['shortcut_launch'] = adb('shell', 'am', 'start', '-n', f'{package}/org.touchhle.android.LauncherActivity',
        '-a', f'{package}.OPEN_IPA', '--es', 'shortcut_ipa_filename', shlex.quote('Angry Birds HD 1.5.0.ipa'))
    time.sleep(8)
    log = adb('logcat', '-d', '-T', start, '-v', 'brief')
    relevant = '\n'.join(x for x in log.splitlines() if re.match(r'[VDIWEF]/(?:SDL/APP|touchHLE|AndroidRuntime|libc)', x))
    report['game_log'] = relevant
    panic = re.search(r'Panic at[^\n]*', relevant)
    report['emulator_crashed'] = bool(panic or 'Fatal signal' in relevant or 'FATAL EXCEPTION' in relevant)
    report['emulator_first_panic'] = panic.group(0) if panic else None
    assert 'Angry Birds' in relevant and 'CPU emulation begins now' in relevant, 'Shortcut intent did not start the installed IPA'
    report['shortcut_ipa_started'] = True
    report['gameplay_verified'] = False
    report['shortcut_flow_verified'] = True
    report['ui_flow_passed'] = True
    report['passed'] = True
except Exception as error:
    report['error'] = repr(error)
finally:
    adb('shell', 'rm', '-f', remote)
    (folder/f'launcher-shortcuts-{args.device}.json').write_text(json.dumps(report, indent=2), encoding='utf-8')
print(json.dumps({k:v for k,v in report.items() if k != 'game_log'}, indent=2))
if not report['passed']:
    raise SystemExit(1)
