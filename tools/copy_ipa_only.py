"""Reuse verified resumable device transfer without launching the IPA."""
from pathlib import Path

template = Path(__file__).with_name('test_ipa_device.py').read_text(encoding='utf-8')
transfer = template.split("installed = adb('shell', 'pm', 'path', package)", 1)[0]
exec(compile(transfer, 'test_ipa_device.py:transfer', 'exec'))
report = dict(identity=identity, ipa=str(source), remote=remote,
              bytes=source.stat().st_size, sha256=digest, transfer_verified=True)
(folder / f'{a.label}.json').write_text(json.dumps(report, indent=2), encoding='utf-8')
print(json.dumps(report, indent=2))
