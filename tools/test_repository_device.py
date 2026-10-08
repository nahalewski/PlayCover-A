"""Verify source-link import, live browsing, search and persistence on comet."""
import datetime
import argparse
import hashlib
import json
from pathlib import Path
import re
import shlex
import subprocess
import time
import urllib.parse
import xml.etree.ElementTree as ET
from adb_device import discover_device

parser = argparse.ArgumentParser()
parser.add_argument('--device', choices=['felix', 'comet'], default='comet')
args = parser.parse_args()
serial = discover_device(args.device)
package = 'org.touchhle.android.a64test'
source = 'https://ipa.cypwn.xyz/cypwn_altstore.json'
link = 'altstore://source?url=' + urllib.parse.quote(source, safe='')
folder = Path('pixel-fold-tests')
report_name = 'repository-device' if args.device == 'comet' else 'repository-gen1'
image_name = 'repository-browser' if args.device == 'comet' else 'repository-gen1-browser'
remote = '/data/local/tmp/playcover-repo-ui.xml'

def adb(*args):
    return subprocess.check_output(['adb', '-s', serial, *args], timeout=40, text=True, encoding='utf-8', errors='replace').strip()

def in_app():
    for attempt in range(12):
        focus = [line for line in adb('shell', 'dumpsys', 'window').splitlines() if 'mCurrentFocus=' in line]
        if any(package in line for line in focus):
            return
        time.sleep(0.5)
    raise RuntimeError('Repository activity is not foreground; stopping UI actions')

def snapshot():
    in_app()
    adb('shell', 'uiautomator', 'dump', remote)
    xml = adb('shell', 'cat', remote)
    return ET.fromstring(xml), xml

def tap(node):
    in_app()
    values = list(map(int, re.findall(r'\d+', node.attrib['bounds'])))
    adb('shell', 'input', 'tap', str((values[0]+values[2])//2), str((values[1]+values[3])//2))

def named(root, text):
    return next(node for node in root.iter('node') if node.attrib.get('text') == text)

report = dict(tested_at=datetime.datetime.now(datetime.timezone.utc).isoformat(), serial=serial,
              package=package, source=source, link=link,
              apk_sha256=hashlib.sha256(Path('PlayCover-A-9Pro-test.apk').read_bytes()).hexdigest(),
              passed=False, ipa_downloaded=False)
try:
    adb('shell', 'am', 'force-stop', package)
    report['launch'] = adb('shell', f'am start -n {package}/org.touchhle.android.LauncherActivity -a android.intent.action.VIEW -d {shlex.quote(link)}')
    time.sleep(2)
    root, _ = snapshot()
    if any(node.attrib.get('text') == 'Add repository' for node in root.iter('node')):
        tap(named(root, 'Add'))
    for attempt in range(8):
        time.sleep(2)
        root, xml = snapshot()
        if any(node.attrib.get('text') == 'CyPwn IPA Library' for node in root.iter('node')):
            break
    else:
        raise RuntimeError('Live repository browser did not open')
    rows = [node for node in root.iter('node') if node.attrib.get('resource-id') == 'android:id/text1']
    if not rows:
        raise RuntimeError('Repository has no visible app rows')
    assert any(node.attrib.get('class') == 'android.widget.ImageView' for node in root.iter('node')), 'App icon views missing'
    for attempt in range(12):
        root, xml = snapshot()
        loaded_icons = [node.attrib.get('content-desc') for node in root.iter('node')
                        if node.attrib.get('class') == 'android.widget.ImageView'
                        and node.attrib.get('content-desc', '').endswith(' icon loaded')]
        if loaded_icons:
            break
        time.sleep(1)
    assert loaded_icons, 'Repository icons did not load'
    query = rows[0].attrib['text'].split(' · ')[0].strip()
    search = next(node for node in root.iter('node') if node.attrib.get('class') == 'android.widget.EditText')
    tap(search)
    # Input a safe word from a known visible app, without shell metacharacters.
    query = next(word for word in re.findall(r'[A-Za-z]+', query) if len(word) > 2)
    adb('shell', 'input', 'text', query)
    time.sleep(1)
    root, xml = snapshot()
    filtered = [node.attrib['text'] for node in root.iter('node') if node.attrib.get('resource-id') == 'android:id/text1']
    assert filtered and all(query.lower() in row.lower() for row in filtered)
    preferences = adb('shell', 'run-as', package, 'cat', 'shared_prefs/launcher.xml')
    saved = next(node.text for node in ET.fromstring(preferences).iter('string') if node.attrib.get('name') == 'repositories')
    assert any(item['url'] == source and item['name'] == 'CyPwn IPA Library' for item in json.loads(saved)), 'Source was not saved'
    (folder/f'{image_name}.xml').write_text(xml)
    in_app()
    screenshot = '/data/local/tmp/playcover-repository.png'
    adb('shell', 'screencap', '-p', screenshot)
    adb('pull', screenshot, str(folder/f'{image_name}.png'))
    adb('shell', 'rm', screenshot)
    adb('shell', 'am', 'force-stop', package)
    adb('shell', f'am start -n {package}/org.touchhle.android.LauncherActivity')
    time.sleep(2)
    root, _ = snapshot()
    tap(named(root, 'Repositories'))
    root, _ = snapshot()
    tap(next(node for node in root.iter('node') if node.attrib.get('text', '').startswith('CyPwn IPA Library\n')))
    for attempt in range(12):
        time.sleep(1)
        root, _ = snapshot()
        if any(node.attrib.get('text') == 'Search apps' for node in root.iter('node')):
            break
    else:
        raise RuntimeError('Saved source did not reopen its app page')
    assert not any(node.attrib.get('text') == 'Add repository' for node in root.iter('node'))
    report.update(saved_source_reopened=True, loaded_icons=loaded_icons, app_icon_views_present=True, passed=True, imported_source_name='CyPwn IPA Library', search_query=query,
                  search_results=filtered, source_persisted=True,
                  screenshot=str(folder/f'{image_name}.png'))
except Exception as error:
    report['error'] = repr(error)
finally:
    adb('shell', 'rm', '-f', remote)
    (folder/f'{report_name}.json').write_text(json.dumps(report, indent=2))
print(json.dumps(report, indent=2))
if not report['passed']:
    raise SystemExit(1)
