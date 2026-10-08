"""Validate official IPA transfers through repository UI; never execute downloaded apps.

Small downloads may finish before a pause/concurrency observation is possible. Such
checks are recorded as unobserved rather than inferred from a completed transfer.
"""
import argparse, datetime, hashlib, json, re, shlex, subprocess, time
from pathlib import Path
import urllib.parse, urllib.request
import xml.etree.ElementTree as ET

p = argparse.ArgumentParser()
p.add_argument('--serial', required=True)
p.add_argument('--report', type=Path, required=True)
p.add_argument('--package', default='org.touchhle.android.a64test')
p.add_argument('--timeout', type=int, default=300)
p.add_argument('--large', action='store_true', help='Use official 80 MB and 18 MB DolphiniOS builds to improve overlap observations')
p.add_argument('--pause-check', action='store_true', help='Attempt accessibility-based pause; may fail Android idle detection during progress animations')
args = p.parse_args()
package = args.package
sources = [('Flycast', 'https://flyinghead.github.io/flycast-builds/altstore.json'),
           ('OpenParsec', 'https://github.com/hugeBlack/OpenParsec/releases/download/nightly/altstore.json')]
if args.large:
    sources = [('DolphiniOS', 'https://altstore.oatmealdome.me'),
               ('DolphiniOS (Public Beta)', 'https://altstore.oatmealdome.me')]
report = {'tested_at': datetime.datetime.now(datetime.timezone.utc).isoformat(),
          'serial': args.serial, 'apps_executed': False, 'observations': []}

def adb(*words, binary=False):
    return subprocess.check_output(['adb', '-s', args.serial, *words], timeout=45,
        text=not binary, **({} if binary else {'encoding':'utf-8', 'errors':'replace'}))

def save():
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, indent=2), encoding='utf-8')

def snapshot():
    adb('shell', 'uiautomator', 'dump', '/data/local/tmp/download-test.xml')
    return ET.fromstring(adb('shell', 'cat', '/data/local/tmp/download-test.xml'))

def find(root, text, prefix=False):
    return next((n for n in root.iter('node') if
        (n.attrib.get('text','').startswith(text) if prefix else n.attrib.get('text') == text)), None)

def tap(node):
    if node is None: raise AssertionError('UI element missing')
    x1,y1,x2,y2 = map(int, re.findall(r'\d+', node.attrib['bounds']))
    adb('shell', 'input', 'tap', str((x1+x2)//2), str((y1+y2)//2))

def jobs():
    try: return json.loads(adb('shell', 'run-as', package, 'cat', 'files/ipa-downloads.json'))
    except subprocess.CalledProcessError: return []

def observation(label):
    current = jobs()
    report['observations'].append({'label':label, 'time':time.time(), 'jobs':[
        {k:j.get(k) for k in ('id','name','status','received','total','target','error')} for j in current]})
    save()
    return current

def open_source(url):
    link = 'altstore://source?url=' + urllib.parse.quote(url, safe='')
    adb('shell', 'am start -n '+package+'/org.touchhle.android.LauncherActivity -a android.intent.action.VIEW -d '+shlex.quote(link))
    for _ in range(35):
        time.sleep(.4); root = snapshot()
        if find(root,'Add') is not None: tap(find(root,'Add')); continue
        if find(root,'All') is not None: return root
        if find(root,'Repository error') is not None: raise AssertionError('Repository error: '+str([n.attrib.get('text') for n in root.iter('node')]))
    raise AssertionError('Feed did not open')

def enqueue(name, url):
    root = open_source(url)
    row = next((n for n in root.iter('node') if n.attrib.get('resource-id') == 'android:id/text1'
                and n.attrib.get('text','').split(' · ',1)[0] == name), None)
    tap(row)
    tap(find(snapshot(),'Download'))
    time.sleep(.2)
    # Active progress animations prevent Android uiautomator from becoming idle.
    # Capture the visible page without waiting for an idle accessibility tree.
    screen = args.report.with_name(args.report.stem+'-'+re.sub('[^a-z0-9]','',name.lower())+'.png')
    screen.write_bytes(adb('exec-out','screencap','-p',binary=True))
    report.setdefault('automatic_library_navigation_screens',[]).append(str(screen.resolve()))

def launch():
    adb('shell','am','start','-n',package+'/org.touchhle.android.LauncherActivity')
    time.sleep(1)

try:
    report['device'] = adb('shell','getprop','ro.product.model').strip()
    before = {j['id'] for j in jobs()}
    launch()
    for name,url in sources:
        enqueue(name,url)
        observation('enqueued '+name)
    current = [j for j in observation('both requested') if j['id'] not in before]
    report['new_ids'] = [j['id'] for j in current]
    report['simultaneous_running_observed'] = len([j for j in current if j['status']=='RUNNING']) >= 2
    active = next((j for j in current if j['status']=='RUNNING' and j['received'] > 0), None) if args.pause_check else None
    if active:
        tap(next((n for n in snapshot().iter('node') if n.attrib.get('content-desc') == active['name']+' download actions'),None))
        tap(find(snapshot(),'Pause download'))
        paused = next(j for j in observation('paused') if j['id']==active['id'])
        report['pause_observed'] = paused['status']=='PAUSED'
        report['paused_bytes'] = paused['received']
        tap(next((n for n in snapshot().iter('node') if n.attrib.get('content-desc') == active['name']+' download actions'),None))
        tap(find(snapshot(),'Resume download'))
        observation('resumed')
    else:
        report['pause_observed'] = False
        report['pause_note'] = 'Pause accessibility check skipped; genuine partial resume is checked by force-stop/reopen' if not args.pause_check else 'No in-progress nonempty transfer remained at UI observation time'
    remaining = [j for j in jobs() if j['id'] in report['new_ids'] and j['status']=='RUNNING']
    report['force_stop_inprogress_ids'] = [j['id'] for j in remaining]
    adb('shell','am','force-stop',package)
    observation('after force stop durable store')
    launch(); observation('after reopen')
    deadline = time.monotonic()+args.timeout
    while time.monotonic()<deadline:
        current = [j for j in jobs() if j['id'] in report['new_ids']]
        if current and all(j['status'] in ('COMPLETE','FAILED','PAUSED') for j in current): break
        time.sleep(1)
    current = [j for j in observation('final') if j['id'] in report['new_ids']]
    report['all_complete'] = len(current)==2 and all(j['status']=='COMPLETE' for j in current)
    report['completed_archives'] = []
    for j in current:
        if j['status'] != 'COMPLETE': continue
        # Only the two newly generated private target files; do not overwrite/delete existing files.
        remote = '/sdcard/Android/data/'+package+'/files/touchHLE_apps/'+j['target']
        sha = adb('shell','sha256sum '+shlex.quote(remote)).split()[0]
        report['completed_archives'].append({'id':j['id'], 'filename':j['target'],
            'bytes':j['received'],'sha256':sha,'validated_before_library_publish':True})
    log = adb('logcat','-d','-s','IpaDownloads:I','*:S')
    relevant = [line for line in log.splitlines() if any(i in line for i in report['new_ids'])]
    report['transfer_logs'] = relevant
    report['resumed_206_observed'] = any(re.search(r'offset=[1-9][0-9]* status=206',line) for line in relevant)
    report['two_worker_log_observed'] = any('active slots=2' in line for line in relevant)
    apkpath=adb('shell','pm','path',package).strip().split('package:',1)[1]
    report['installed_apk_sha256']=adb('shell','sha256sum '+shlex.quote(apkpath)).split()[0]
    report['passed']=report['all_complete'] and report['two_worker_log_observed'] and report['resumed_206_observed']
    screenshot = args.report.with_suffix('.png')
    screenshot.write_bytes(adb('exec-out','screencap','-p',binary=True))
    report['screenshot'] = str(screenshot.resolve())
except Exception as error:
    report['error'] = str(error)
finally:
    save()
print(json.dumps(report,indent=2))
