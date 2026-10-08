"""Local authenticated ipatool companion. Apple credentials stay in ipatool's PC prompt."""
import argparse
import getpass
import hashlib
import hmac
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import re
import secrets
import subprocess
import threading
import time
import zipfile
import warnings
from urllib.parse import parse_qs, urlsplit

class Backend:
    def __init__(self, executable, downloads, keychain_passphrase=None):
        self.executable = str(Path(executable).resolve())
        self.downloads = Path(downloads).resolve()
        self.downloads.mkdir(parents=True, exist_ok=True)
        self.lock = threading.Lock()
        self.jobs = {}
        self.login = None
        self._keychain_passphrase = keychain_passphrase

    def child_environment(self):
        environment = os.environ.copy()
        environment.pop('IPATOOL_KEYCHAIN_PASSPHRASE', None)
        if self._keychain_passphrase is not None:
            environment['IPATOOL_KEYCHAIN_PASSPHRASE'] = self._keychain_passphrase
        return environment

    def run(self, args, timeout=120):
        if self.login is not None and self.login.poll() is None:
            raise RuntimeError('Complete the local computer login prompt first')
        with self.lock:
            result = subprocess.run([self.executable, '--format', 'json', '--non-interactive', *args],
                stdin=subprocess.DEVNULL, capture_output=True, timeout=timeout, env=self.child_environment(), creationflags=getattr(subprocess, 'CREATE_NO_WINDOW', 0))
        # Never forward raw CLI errors/logs; they may contain account information.
        records = []
        for line in (result.stdout + result.stderr).decode('utf-8', errors='replace').splitlines():
            try:
                item = json.loads(line)
                if isinstance(item, dict): records.append(item)
            except ValueError: pass
        if result.returncode:
            raise RuntimeError('ipatool request failed; sign in locally and verify ownership/availability')
        return records

    def status(self):
        pending = self.login is not None and self.login.poll() is None
        authenticated = False
        if not pending:
            try: authenticated = any(r.get('success') is True for r in self.run(['auth', 'info']))
            except (OSError, RuntimeError, subprocess.TimeoutExpired): pass
        return dict(available=Path(self.executable).is_file(), authenticated=authenticated, login_pending=pending)

    def open_login(self):
        if os.name != 'nt':
            raise RuntimeError('Run ipatool auth login in a local terminal on this computer')
        if self.login is None or self.login.poll() is not None:
            # Interactive console masks password; neither password nor 2FA enters argv/API.
            self.login = subprocess.Popen([self.executable, 'auth', 'login'], env=self.child_environment(), creationflags=subprocess.CREATE_NEW_CONSOLE)
        return dict(login_pending=True, message='Complete Apple Account login and 2FA in the computer console, then refresh status')

    def apps(self, args):
        records = self.run(args)
        record = next((r for r in reversed(records) if isinstance(r.get('apps'), list)), None)
        if record is None: raise RuntimeError('ipatool returned no app list')
        return {k: record[k] for k in ('apps', 'count', 'totalCount', 'page') if k in record}

    def download(self, app_id):
        if not isinstance(app_id, int) or isinstance(app_id, bool) or not 0 < app_id < 2**63:
            raise ValueError('app_id must be a positive App Store numeric ID')
        if len(self.jobs) >= 128: raise RuntimeError('Download job limit reached; restart companion after finishing jobs')
        job_id = secrets.token_hex(16)
        job = dict(id=job_id, status='queued', bytes=0)
        self.jobs[job_id] = job
        def worker():
            target = self.downloads / (job_id + '.ipa')
            try:
                job['status'] = 'downloading'
                records = self.run(['download', '--app-id', str(app_id), '--platform', 'iphone', '--output', str(target)], timeout=3600)
                if not any(r.get('success') is True for r in records) or not target.is_file() or not target.stat().st_size:
                    raise RuntimeError('ipatool did not produce a complete IPA')
                with zipfile.ZipFile(target) as archive:
                    if not any(n.startswith('Payload/') and n.endswith('.app/Info.plist') for n in archive.namelist()) or archive.testzip() is not None:
                        raise RuntimeError('Downloaded file is not an intact iOS IPA')
                digest = hashlib.sha256()
                with target.open('rb') as stream:
                    for block in iter(lambda: stream.read(1024*1024), b''): digest.update(block)
                job.update(status='complete', bytes=target.stat().st_size, sha256=digest.hexdigest(), download_path='/v1/files/' + job_id)
            except Exception:
                job.update(status='failed', error='Download failed; verify local sign-in, ownership and availability in ipatool')
        threading.Thread(target=worker, daemon=True).start()
        return job.copy()

def server(backend, token, host, port):
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_): pass
        def send_json(self, code, value):
            data = json.dumps(value).encode()
            self.send_response(code); self.send_header('Content-Type', 'application/json'); self.send_header('Cache-Control','no-store')
            self.send_header('Content-Length', str(len(data))); self.end_headers(); self.wfile.write(data)
        def authorized(self):
            return hmac.compare_digest(self.headers.get('Authorization',''), 'Bearer ' + token)
        def do_GET(self):
            if not self.authorized(): return self.send_json(401, {'error':'Companion token required'})
            url = urlsplit(self.path); query = parse_qs(url.query)
            try:
                if url.path == '/v1/status': return self.send_json(200, backend.status())
                if url.path == '/v1/search':
                    term = query.get('q',[''])[0]
                    if not term.strip() or len(term)>200 or term.startswith('-'): raise ValueError('Invalid search term')
                    limit = max(1,min(50,int(query.get('limit',['25'])[0])))
                    return self.send_json(200,backend.apps(['search',term,'--limit',str(limit),'--platform','iphone']))
                if url.path == '/v1/purchases':
                    page = max(1,min(1000,int(query.get('page',['1'])[0])))
                    return self.send_json(200,backend.apps(['list-purchases','--page',str(page),'--max-results','50','--platform','iphone']))
                match = re.fullmatch(r'/v1/(downloads|files)/([0-9a-f]{32})',url.path)
                if match:
                    kind, job_id = match.groups(); job = backend.jobs.get(job_id)
                    if job is None: return self.send_json(404, {'error':'Unknown download'})
                    if kind == 'downloads':
                        view = job.copy(); target = backend.downloads/(job_id+'.ipa')
                        if view['status']=='downloading' and target.exists(): view['bytes']=target.stat().st_size
                        return self.send_json(200,view)
                    if job['status']!='complete': return self.send_json(409,{'error':'Download not complete'})
                    target = backend.downloads/(job_id+'.ipa')
                    self.send_response(200); self.send_header('Content-Type','application/octet-stream'); self.send_header('Content-Length',str(target.stat().st_size)); self.end_headers()
                    with target.open('rb') as stream:
                        for chunk in iter(lambda: stream.read(1024*1024),b''): self.wfile.write(chunk)
                    return
                self.send_json(404, {'error':'Unknown endpoint'})
            except (ValueError, OSError, RuntimeError, subprocess.TimeoutExpired): self.send_json(400,{'error':'Request failed; check local login, query and app ownership'})
        def do_POST(self):
            if not self.authorized(): return self.send_json(401,{'error':'Companion token required'})
            try:
                size=int(self.headers.get('Content-Length','0'))
                if not 0<=size<=4096: raise ValueError('Request too large')
                body=json.loads(self.rfile.read(size) or b'{}')
                if not isinstance(body,dict): raise ValueError('Object required')
                if self.path=='/v1/login':
                    if body: raise ValueError('Credentials are accepted only in local console')
                    return self.send_json(200,backend.open_login())
                if self.path=='/v1/downloads':
                    if set(body)!={'app_id'}: raise ValueError('Only existing-owned downloads are supported')
                    return self.send_json(202,backend.download(body['app_id']))
                self.send_json(404,{'error':'Unknown endpoint'})
            except (ValueError, OSError, RuntimeError): self.send_json(400,{'error':'Request invalid; login credentials stay on computer and downloads require ownership'})
    return ThreadingHTTPServer((host,port),Handler)

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--ipatool',default=str(Path(__file__).with_name('ipatool.exe')))
    parser.add_argument('--port',type=int,default=18765)
    parser.add_argument('--token-file',type=Path,default=Path.home()/'.playcover-a'/'companion-token')
    parser.add_argument('--downloads',type=Path,default=Path.home()/'.playcover-a'/'downloads')
    parser.add_argument('--unlock-keychain',action='store_true',help='Prompt locally for the keychain unlock passphrase; never an Apple password')
    args=parser.parse_args()
    args.token_file.parent.mkdir(parents=True,exist_ok=True)
    if not args.token_file.exists():
        descriptor=os.open(args.token_file,os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600)
        with os.fdopen(descriptor,'w') as stream: stream.write(secrets.token_urlsafe(32))
    token=args.token_file.read_text().strip()
    if len(token)<32: parser.error('Companion token must have at least 32 characters')
    unlock = None
    if args.unlock_keychain:
        try:
            with warnings.catch_warnings():
                warnings.simplefilter('error', getpass.GetPassWarning)
                unlock = getpass.getpass('Local ipatool keychain passphrase (not Apple password): ')
            if not unlock: parser.error('Keychain passphrase cannot be empty')
        except (getpass.GetPassWarning, EOFError):
            parser.error('Keychain unlock requires a local masked terminal prompt')
    backend=Backend(args.ipatool,args.downloads,keychain_passphrase=unlock)
    if not Path(backend.executable).is_file(): parser.error('ipatool binary missing')
    print('Companion: http://127.0.0.1:'+str(args.port)+'; token file: '+str(args.token_file),flush=True)
    server(backend,token,'127.0.0.1',args.port).serve_forever()

if __name__=='__main__': main()
