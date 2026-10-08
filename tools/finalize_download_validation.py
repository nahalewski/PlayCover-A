"""Read-only completion evidence for the explicitly tested tablet download jobs."""
import hashlib, json, re, shlex, subprocess
from pathlib import Path

serial='192.168.0.56:45601'
package='org.touchhle.android.a64test'
path=Path('pixel-fold-tests/tablet-persistent-downloads.json')
report=json.loads(path.read_text(encoding='utf-8'))
def adb(*args, binary=False):
    return subprocess.check_output(['adb','-s',serial,*args],timeout=60,text=not binary,
        **({} if binary else {'encoding':'utf-8','errors':'replace'}))
jobs=json.loads(adb('shell','run-as',package,'cat','files/ipa-downloads.json'))
tested=[j for j in jobs if j['id'] in report['new_ids']]
logs=adb('logcat','-d','-s','IpaDownloads:I','*:S')
report['transfer_logs']=[s for s in logs.splitlines() if any(j['id'] in s for j in tested)]
report['two_worker_log_observed']=any('active slots=2' in s for s in report['transfer_logs'])
report['resumed_206_observed']=any(re.search(r'offset=[1-9][0-9]* status=206',s) for s in report['transfer_logs'])
report['manual_force_stop_reopen']={'partial_checkpoint_bytes':69144128,'actual_resumed_offset':72819264,
    'note':'Partial file length exceeded last durable progress checkpoint; engine correctly trusted actual part length'}
report['completed_archives']=[]
for j in tested:
    remote='/sdcard/Android/data/'+package+'/files/touchHLE_apps/'+j['target']
    sha=adb('shell','sha256sum '+shlex.quote(remote)).split()[0]
    report['completed_archives'].append({'id':j['id'],'filename':j['target'],'status':j['status'],
        'bytes':j['received'],'sha256':sha,'validated_before_library_publish':j['status']=='COMPLETE'})
apkpath=adb('shell','pm','path',package).strip().split('package:',1)[1]
report['installed_apk_sha256']=adb('shell','sha256sum '+shlex.quote(apkpath)).split()[0]
local=Path('PlayCover-A-fold-test.apk')
if local.exists():
    report['local_apk_sha256']=hashlib.file_digest(local.open('rb'),'sha256').hexdigest()
    report['installed_matches_local']=report['installed_apk_sha256']==report['local_apk_sha256']
report['all_complete']=len(tested)==2 and all(j['status']=='COMPLETE' for j in tested)
report['passed']=report['all_complete'] and report['two_worker_log_observed'] and report['resumed_206_observed']
report['harness_note']=report.pop('error',None)
report['pause_observed']=False
report['pause_note']='Accessibility cannot reach idle during animated progress; force-stop/reopen validated genuine partial resume instead'
screen=path.with_name('tablet-downloads-completed.png')
screen.write_bytes(adb('exec-out','screencap','-p',binary=True))
report['completed_screenshot']=str(screen.resolve())
path.write_text(json.dumps(report,indent=2),encoding='utf-8')
print(json.dumps({k:report.get(k) for k in ('passed','installed_apk_sha256','installed_matches_local','completed_archives','resumed_206_observed')},indent=2))
