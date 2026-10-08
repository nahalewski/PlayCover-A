import io
from pathlib import Path
import tempfile
import unittest
from apple_store_transfer import stream_completed_ipa

class Handler:
    def __init__(self, headers): self.headers=headers; self.sent={}; self.wfile=io.BytesIO()
    def send_response(self, status): self.status=status
    def send_header(self, name, value): self.sent[name]=value
    def end_headers(self): pass

class Tests(unittest.TestCase):
    def test_resume_and_if_range_identity(self):
        with tempfile.TemporaryDirectory() as directory:
            file=Path(directory)/'complete.ipa';file.write_bytes(b'0123456789')
            h=Handler({'Range':'bytes=4-','If-Range':'"known"'});stream_completed_ipa(h,file,'known')
            self.assertEqual(h.status,206);self.assertEqual(h.sent['Content-Range'],'bytes 4-9/10');self.assertEqual(h.wfile.getvalue(),b'456789')
            h=Handler({'Range':'bytes=4-','If-Range':'"changed"'});stream_completed_ipa(h,file,'known')
            self.assertEqual(h.status,200);self.assertEqual(h.wfile.getvalue(),b'0123456789')
    def test_reject_invalid_ranges(self):
        with tempfile.TemporaryDirectory() as directory:
            file=Path(directory)/'complete.ipa';file.write_bytes(b'123')
            for value in ('bytes=3-','bytes=99-','bytes=-1','bytes=1-2,4-5'):
                h=Handler({'Range':value});stream_completed_ipa(h,file,'known')
                self.assertEqual(h.status,416);self.assertEqual(h.wfile.getvalue(),b'')
if __name__=='__main__':unittest.main()
