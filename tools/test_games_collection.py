"""Verify the bundled game catalog UI without downloading or running an IPA."""
import argparse, datetime, hashlib, json, re, subprocess, time
from pathlib import Path
import xml.etree.ElementTree as ET

p = argparse.ArgumentParser()
p.add_argument('--serial', required=True)
p.add_argument('--report', required=True)
args = p.parse_args()
package = 'org.touchhle.android.a64test'
feed_path = Path('touchHLE-src/android/app/src/main/assets/games.json')
feed_bytes = feed_path.read_bytes()
feed = json.loads(feed_bytes)
first_name = feed['apps'][0]['name']
report = dict(tested_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),
              serial=args.serial, passed=False, ipa_downloaded=False, gameplay_verified=False,
              catalog_name=feed['name'], catalog_app_count=len(feed['apps']),
              catalog_sha256=hashlib.sha256(feed_bytes).hexdigest(), first_game=first_name)

def adb(*words):
    return subprocess.check_output(['adb', '-s', args.serial, *words], timeout=50,
                                   text=True, encoding='utf-8', errors='replace').strip()

def snapshot():
    for _ in range(12):
        if any(package in line for line in adb('shell', 'dumpsys', 'window').splitlines()
               if 'mCurrentFocus=' in line): break
        time.sleep(0.5)
    else: raise RuntimeError('Launcher is not foreground')
    adb('shell', 'uiautomator', 'dump', '/data/local/tmp/games-collection.xml')
    return ET.fromstring(adb('shell', 'cat', '/data/local/tmp/games-collection.xml'))

def node(root, label):
    return next(n for n in root.iter('node') if n.attrib.get('text') == label)

def tap(n):
    b = list(map(int, re.findall(r'\d+', n.attrib['bounds'])))
    adb('shell', 'input', 'tap', str((b[0]+b[2])//2), str((b[1]+b[3])//2))

def text_list(root):
    return [n.attrib['text'] for n in root.iter('node') if n.attrib.get('text')]

def has_game(root):
    return any(text.startswith(first_name + ' · ') for text in text_list(root))

def open_collection():
    adb('shell', 'am', 'force-stop', package)
    adb('shell', 'am', 'start', '-n', package + '/org.touchhle.android.LauncherActivity')
    root = snapshot()
    tap(node(root, 'Repositories'))
    tap(node(snapshot(), 'Games collection'))
    for _ in range(12):
        root = snapshot()
        if feed['name'] in text_list(root): return root
        time.sleep(0.5)
    raise AssertionError('Bundled catalog did not open')

try:
    installed_apk = adb('shell', 'pm', 'path', package).splitlines()[0].removeprefix('package:')
    report['apk_sha256'] = adb('shell', 'sha256sum', installed_apk).split()[0]
    root = open_collection()
    node(root, 'All'); node(root, 'Games')
    tap(node(root, 'All'))
    root = snapshot()
    assert has_game(root), 'Generated first game is missing'
    report['catalog_opened'] = True
    tap(node(root, 'Games'))
    root = snapshot()
    assert has_game(root), 'Games filter excluded the generated game'
    report['games_filter_verified'] = True
    for _ in range(6):
        loaded = [n.attrib.get('content-desc') for n in root.iter('node')
                  if n.attrib.get('content-desc', '').endswith('icon loaded')]
        if loaded: break
        time.sleep(1); root = snapshot()
    report['icons_loaded'] = loaded
    assert loaded, 'No visible game icon loaded within the observation window'
    search = next(n for n in root.iter('node') if n.attrib.get('class') == 'android.widget.EditText')
    tap(search)
    # First token is enough to test name matching, avoiding Android shell quoting.
    query = re.search(r'[A-Za-z0-9]+', first_name).group(0)
    adb('shell', 'input', 'text', query)
    adb('shell', 'input', 'keyevent', '4')  # Hide IME so the result row is visible.
    root = snapshot()
    assert has_game(root), 'Search did not retain the expected game'
    report['search_match_verified'] = True
    search = next(n for n in root.iter('node') if n.attrib.get('class') == 'android.widget.EditText')
    tap(search)
    adb('shell', 'input', 'keyevent', *(['67'] * len(query)))
    adb('shell', 'input', 'text', 'zzzznomatchcatalog')
    adb('shell', 'input', 'keyevent', '4')
    root = snapshot()
    node(root, 'No apps match these filters.')
    report['search_no_results_verified'] = True
    root = open_collection()
    assert has_game(root), 'Catalog failed to reopen after restart'
    report['reopens_after_restart'] = True
    report['visible_text_after_restart'] = text_list(root)
    report['passed'] = True
except Exception as error:
    report['error'] = repr(error)
finally:
    Path(args.report).write_text(json.dumps(report, indent=2), encoding='utf-8')
print(json.dumps(report, indent=2))
if not report['passed']: raise SystemExit(1)
