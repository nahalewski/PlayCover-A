"""Acquire two version-isolated Apple IPSWs using the verified downloader."""
from pathlib import Path

original = Path(__file__).with_name('obtain_ios_firmware.py').read_text(encoding='utf-8')
for version in ('17.7', '18.0'):
    code = original.replace("root / 'firmware'", "root / 'firmware-ios" + version.split('.')[0] + "'")
    code = code.replace('iPhone10,4?type=ipsw', 'iPhone12,1?type=ipsw')
    code = code.replace("f['version'].startswith('16.7.')", "f['version'] == '" + version + "'")
    exec(compile(code, 'obtain_ios_firmware.py', 'exec'), {'__file__': str(Path(__file__).with_name('obtain_ios_firmware.py'))})
