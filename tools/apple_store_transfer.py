"""Authenticated handler helper for immutable, completed IPA range transfer."""
import re

def stream_completed_ipa(handler, target, sha256):
    length = target.stat().st_size
    start = 0
    requested = handler.headers.get('Range')
    if requested and handler.headers.get('If-Range', '"'+sha256+'"') == '"'+sha256+'"':
        match = re.fullmatch(r'bytes=(\d+)-', requested)
        if not match or int(match[1]) >= length:
            handler.send_response(416); handler.send_header('Content-Range', 'bytes */'+str(length)); handler.send_header('Content-Length','0'); handler.end_headers(); return
        start = int(match[1])
    handler.send_response(206 if start else 200)
    handler.send_header('Content-Type','application/octet-stream')
    handler.send_header('Content-Length',str(length-start))
    handler.send_header('Accept-Ranges','bytes')
    handler.send_header('ETag','"'+sha256+'"')
    if start: handler.send_header('Content-Range',f'bytes {start}-{length-1}/{length}')
    handler.end_headers()
    with target.open('rb') as stream:
        stream.seek(start)
        for chunk in iter(lambda:stream.read(1024*1024),b''): handler.wfile.write(chunk)
