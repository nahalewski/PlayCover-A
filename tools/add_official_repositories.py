"""Add developer-published emulator feeds through the launcher, never download IPAs."""
import argparse, datetime, json, re, shlex, subprocess, time
from pathlib import Path
import urllib.parse, urllib.request
import xml.etree.ElementTree as ET

p = argparse.ArgumentParser()
p.add_argument('--serial', required=True)
p.add_argument('--report', required=True)
p.add_argument('--sources-json', type=Path, help='JSON list of HTTPS URLs or {url,name} entries; also accepts {sources:[...]} or {repositories:[...]}')
p.add_argument('--quick', action='store_true', help='Verify addition and persistence without desktop fetches, category or search tests')
args = p.parse_args()
package = 'org.touchhle.android.a64test'
sources = ['https://provenance-emu.com/apps.json',
           'https://flyinghead.github.io/flycast-builds/altstore.json',
           'https://altstore.oatmealdome.me']
source_names = {}
if args.sources_json:
    try:
        entries = json.loads(args.sources_json.read_text(encoding='utf-8'))
        if isinstance(entries, dict):
            entries = entries.get('repositories', entries.get('sources'))
        if not isinstance(entries, list):
            raise ValueError('Expected a JSON list, repositories list or sources list')
        sources = []
        for entry in entries:
            url = entry if isinstance(entry, str) else entry.get('url') if isinstance(entry, dict) else None
            if not isinstance(url, str):
                raise ValueError('Every source needs a URL string')
            parsed = urllib.parse.urlsplit(url)
            if parsed.scheme != 'https' or not parsed.hostname or parsed.username or parsed.password or parsed.fragment:
                raise ValueError('Source must be an HTTPS URL without credentials or fragment: ' + url)
            if url not in sources:
                sources.append(url)
            if isinstance(entry, dict) and isinstance(entry.get('name'), str):
                source_names[url] = entry['name']
        if not sources:
            raise ValueError('Source list is empty')
    except (OSError, ValueError) as error:
        p.error(str(error))
report = dict(tested_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),
              serial=args.serial, ipa_downloaded=False, quick=args.quick, sources=[])

def adb(*words):
    return subprocess.check_output(['adb', '-s', args.serial, *words], timeout=45,
                                   text=True, encoding='utf-8', errors='replace').strip()

def snapshot():
    for _ in range(12):
        focus = adb('shell', 'dumpsys', 'window')
        if any(package in line for line in focus.splitlines() if 'mCurrentFocus=' in line):
            break
        time.sleep(0.5)
    else:
        raise RuntimeError('Launcher not foreground')
    adb('shell', 'uiautomator', 'dump', '/data/local/tmp/official-repos.xml')
    return ET.fromstring(adb('shell', 'cat', '/data/local/tmp/official-repos.xml'))

def texts(root):
    return [n.attrib.get('text', '') for n in root.iter('node') if n.attrib.get('text')]

def tap(node):
    b = list(map(int, re.findall(r'\d+', node.attrib['bounds'])))
    adb('shell', 'input', 'tap', str((b[0]+b[2])//2), str((b[1]+b[3])//2))

def find(root, label):
    return next((n for n in root.iter('node') if n.attrib.get('text') == label), None)

def save():
    Path(args.report).parent.mkdir(parents=True, exist_ok=True)
    Path(args.report).write_text(json.dumps(report, indent=2), encoding='utf-8')

report['device'] = adb('shell', 'getprop', 'ro.product.device')
adb('shell', 'am', 'force-stop', package)
for url in sources:
    item = dict(url=url, added=False)
    if url in source_names:
        item['listed_name'] = source_names[url]
    report['sources'].append(item)
    try:
        if not args.quick:
            request = urllib.request.Request(url, headers={'User-Agent': 'Mozilla/5.0'})
            with urllib.request.urlopen(request, timeout=25) as response:
                feed = json.load(response)
            item.update(feed_name=feed.get('name'), published_app_count=len(feed.get('apps', [])))
    except Exception as error:
        item['desktop_fetch_error'] = str(error)
    try:
        link = 'altstore://source?url=' + urllib.parse.quote(url, safe='')
        adb('shell', 'am start -n ' + package + '/org.touchhle.android.LauncherActivity -a android.intent.action.VIEW -d ' + shlex.quote(link))
        time.sleep(1)
        root = snapshot()
        if find(root, 'Add') is not None:
            tap(find(root, 'Add'))
        for _ in range(12):
            time.sleep(1)
            root = snapshot()
            if find(root, 'All') is not None or find(root, 'Repository error') is not None:
                break
        item['visible_text'] = texts(root)
        if find(root, 'Repository error') is not None:
            item['error'] = texts(root)
            tap(find(root, 'OK'))
        else:
            assert find(root, 'All') is not None, 'Repository did not finish loading'
            if args.quick:
                item['added'] = True
                save()
                print(json.dumps(item), flush=True)
                continue
            tap(find(root, 'All'))
            root = snapshot()
            item['visible_text'] = texts(root)
            searches = [n for n in root.iter('node') if n.attrib.get('class') == 'android.widget.EditText']
            if searches and searches[0].attrib.get('text') not in ('', 'Search apps'):
                tap(searches[0])
                adb('shell', 'input', 'keyevent', '123')
                adb('shell', 'input', 'keyevent', *(['67'] * len(searches[0].attrib.get('text', ''))))
                adb('shell', 'input', 'keyevent', '4')
                root = snapshot()
                item['visible_text'] = texts(root)
            item['added'] = True
            tap(find(root, 'Emulators'))
            root = snapshot()
            item['emulator_filter_text'] = texts(root)
            item['icons_loaded'] = [n.attrib.get('content-desc') for n in root.iter('node')
                                    if n.attrib.get('content-desc', '').endswith('icon loaded')]
            searches = [n for n in root.iter('node') if n.attrib.get('class') == 'android.widget.EditText']
            if searches:
                tap(searches[0]); adb('shell', 'input', 'text', 'zzzznomatch')
                root = snapshot()
                item['search_composes_with_category'] = find(root, 'No apps match these filters.') is not None
                adb('shell', 'input', 'keyevent', *(['67'] * len('zzzznomatch')))
                adb('shell', 'input', 'keyevent', '4')
    except Exception as error:
        item['ui_error'] = str(error)
    save()
    print(json.dumps(item), flush=True)
adb('shell', 'am', 'force-stop', package)
adb('shell', 'am', 'start', '-n', package + '/org.touchhle.android.LauncherActivity')
time.sleep(1)
root = snapshot()
tap(find(root, 'Repositories'))
root = snapshot()
report['sources_after_restart'] = texts(root)
prefs = adb('shell', 'run-as', package, 'cat', 'shared_prefs/launcher.xml')
entries = ET.fromstring(prefs).find("string[@name='repositories']")
saved_urls = [entry['url'] for entry in json.loads(entries.text)] if entries is not None else []
report['persisted_source_urls'] = [url for url in sources if url in saved_urls]
for item in report['sources']:
    item['persisted'] = item['url'] in saved_urls
report['all_requested_persisted'] = all(url in saved_urls for url in sources)
report['all_added_persisted'] = all(not x['added'] or x['url'] in report['persisted_source_urls'] for x in report['sources'])
save()
print(json.dumps({'persisted': report['persisted_source_urls']}), flush=True)
