"""Verify an explicit empty Android emulator launch returns to the modern library."""
import argparse,json,subprocess,time
from pathlib import Path
parser=argparse.ArgumentParser()
parser.add_argument('--serial',required=True)
args=parser.parse_args()
prefix=['adb','-s',args.serial]
package='org.touchhle.android.a64test'
def adb(*words):
    return subprocess.check_output(prefix+list(words),text=True,encoding='utf-8',errors='replace',timeout=30)
adb('shell','am','force-stop',package)
adb('shell','am','start','-n',package+'/org.touchhle.android.MainActivity')
time.sleep(3)
focus=[line.strip() for line in adb('shell','dumpsys','window').splitlines() if 'mCurrentFocus=' in line]
assert any('LauncherActivity' in line for line in focus),focus
adb('shell','uiautomator','dump','/data/local/tmp/empty-main-launch.xml')
xml=adb('shell','cat','/data/local/tmp/empty-main-launch.xml')
assert 'Installed IPAs' in xml and 'Repositories' in xml and 'Settings' in xml
report={'serial':args.serial,'focus':focus,'modern_library_visible':True,'missing_app_did_not_show_legacy_picker':True}
Path('pixel-fold-tests/empty-main-launch.json').write_text(json.dumps(report,indent=2),encoding='utf-8')
print(json.dumps(report))
