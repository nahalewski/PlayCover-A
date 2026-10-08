"""Durable, idempotent companion jobs. Integrate using durable_backend(Backend)."""
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import threading
import zipfile

class JobStore:
    def __init__(self, path):
        self.path = Path(path)
        self.lock = threading.RLock()
        self.jobs = {}
        if self.path.exists():
            records = json.loads(self.path.read_text())
            if not isinstance(records, dict) or len(records) > 128:
                raise ValueError('Invalid persisted companion jobs')
            for key, job in records.items():
                if not re.fullmatch('[0-9a-f]{32}', key) or job.get('id') != key or not isinstance(job.get('app_id'), int):
                    raise ValueError('Invalid persisted companion job')
            self.jobs = records
    def save(self):
        with self.lock:
            self.path.parent.mkdir(parents=True, exist_ok=True)
            temporary = self.path.with_suffix('.writing')
            with temporary.open('w', encoding='utf-8') as stream:
                json.dump(self.jobs, stream); stream.flush(); os.fsync(stream.fileno())
            os.replace(temporary, self.path)
    def submit(self, app_id, request_id=None):
        if not isinstance(app_id, int) or isinstance(app_id, bool) or not 0 < app_id < 2**63:
            raise ValueError('Invalid app ID')
        if request_id is not None and not re.fullmatch('[0-9a-f]{32}', request_id):
            raise ValueError('Invalid idempotency key')
        with self.lock:
            for job in self.jobs.values():
                if request_id and job.get('request_id') == request_id:
                    if job['app_id'] != app_id: raise ValueError('Idempotency key belongs to another app')
                    return job.copy(), False
                if job['app_id'] == app_id and job['status'] in ('queued', 'downloading', 'waiting_for_login'):
                    return job.copy(), False
            if len(self.jobs) >= 128: raise ValueError('Job limit reached')
            identity = secrets.token_hex(16)
            job = dict(id=identity, app_id=app_id, request_id=request_id, status='queued', bytes=0)
            self.jobs[identity] = job; self.save()
            return job.copy(), True
    def update(self, identity, **fields):
        with self.lock:
            self.jobs[identity].update(fields); self.save()
    def pending(self):
        with self.lock:
            return [j.copy() for j in self.jobs.values() if j['status'] in ('queued','downloading','waiting_for_login')]

def durable_backend(base):
    class DurableBackend(base):
        def __init__(self, executable, downloads, keychain_passphrase=None):
            super().__init__(executable, downloads, keychain_passphrase=keychain_passphrase)
            self.store = JobStore(self.downloads.parent / 'download-jobs.json')
            self.jobs = self.store.jobs
            self.running = set()
            self.job_lock = threading.Lock()
            # Do not fail queued jobs merely because sign-in has expired.
            for job in self.store.pending(): self.store.update(job['id'], status='waiting_for_login')
        def status(self):
            state = super().status()
            if state['authenticated']:
                for job in self.store.pending(): self._start(job['id'])
            return state
        def download(self, app_id, request_id=None):
            job, _ = self.store.submit(app_id, request_id)
            if job['status'] not in ('complete','failed'): self._start(job['id'])
            return self.jobs[job['id']].copy()
        def _start(self, identity):
            with self.job_lock:
                if identity in self.running: return
                self.running.add(identity)
            threading.Thread(target=self._worker, args=(identity,), daemon=True).start()
        def _worker(self, identity):
            job = self.jobs[identity]
            target = self.downloads / (identity + '.ipa')
            try:
                self.store.update(identity, status='downloading')
                # The pinned upstream validates HTTP ranges and resumes its own
                # existing output. Preserve both the IPA and upstream .tmp file.
                records = self.run(['download','--app-id',str(job['app_id']),'--platform','iphone','--output',str(target)],timeout=3600)
                if not any(r.get('success') is True for r in records): raise RuntimeError('Download incomplete')
                with zipfile.ZipFile(target) as archive:
                    if not any(n.startswith('Payload/') and n.endswith('.app/Info.plist') for n in archive.namelist()) or archive.testzip() is not None:
                        raise RuntimeError('Invalid IPA')
                digest = hashlib.sha256()
                with target.open('rb') as stream:
                    for block in iter(lambda:stream.read(1024*1024),b''): digest.update(block)
                self.store.update(identity,status='complete',bytes=target.stat().st_size,sha256=digest.hexdigest(),download_path='/v1/files/'+identity)
            except Exception:
                self.store.update(identity,status='failed',error='Owned app download failed; check local account and availability')
            finally:
                with self.job_lock: self.running.discard(identity)
    return DurableBackend

if __name__ == '__main__':
    import tempfile
    import unittest
    class Tests(unittest.TestCase):
        def test_restart_identity_and_completed_idempotency(self):
            with tempfile.TemporaryDirectory() as directory:
                path=Path(directory)/'jobs.json'; store=JobStore(path)
                first,new=store.submit(42,'a'*32); self.assertTrue(new)
                store.update(first['id'],status='complete',sha256='known')
                recovered=JobStore(path)
                again,new=recovered.submit(42,'a'*32)
                self.assertFalse(new); self.assertEqual(again['id'],first['id'])
                self.assertEqual(again['sha256'],'known')
                with self.assertRaises(ValueError): recovered.submit(99,'a'*32)
        def test_concurrent_duplicate_and_pending_restart(self):
            with tempfile.TemporaryDirectory() as directory:
                path=Path(directory)/'jobs.json'; store=JobStore(path); results=[]
                threads=[threading.Thread(target=lambda:results.append(store.submit(42)[0]['id'])) for _ in range(12)]
                for t in threads:t.start()
                for t in threads:t.join()
                self.assertEqual(len(set(results)),1)
                self.assertEqual(JobStore(path).pending()[0]['id'],results[0])
        def test_invalid_keys_and_corrupt_state_preserved(self):
            with tempfile.TemporaryDirectory() as directory:
                path=Path(directory)/'jobs.json'; store=JobStore(path)
                for app in (True,-1,'42'):
                    with self.assertRaises(ValueError):store.submit(app)
                with self.assertRaises(ValueError):store.submit(42,'../../file')
                path.write_text('corrupt')
                with self.assertRaises(ValueError):JobStore(path)
                self.assertEqual(path.read_text(),'corrupt')
    unittest.main()
