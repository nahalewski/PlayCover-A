"""Exercise localhost API rejection paths without authenticating or purchasing."""
import hashlib
import json
from pathlib import Path
import urllib.error
import urllib.request

token = (Path.home() / '.playcover-a/companion-token').read_text().strip()
base = 'http://127.0.0.1:18765'
headers = {'Authorization': 'Bearer ' + token, 'Content-Type': 'application/json'}
checks = [('/v1/status', None, {}, 401),
          ('/v1/downloads', {'app_id': 1, 'acquire_license': True}, headers, 400),
          ('/v1/login', {'password': 'synthetic-test'}, headers, 400)]
results = []
for path, body, request_headers, expected in checks:
    try:
        request = urllib.request.Request(base + path, data=None if body is None else json.dumps(body).encode(), headers=request_headers)
        urllib.request.urlopen(request, timeout=10)
        raise AssertionError('Request unexpectedly accepted')
    except urllib.error.HTTPError as error:
        assert error.code == expected, (path, error.code)
        results.append({'route': path, 'status': error.code})
results.append({'ipatool_sha256': hashlib.sha256(Path('tools/ipatool.exe').read_bytes()).hexdigest()})
Path('pixel-fold-tests/apple-store-companion-api-smoke.json').write_text(json.dumps(results, indent=2), encoding='utf-8')
print(json.dumps(results))
