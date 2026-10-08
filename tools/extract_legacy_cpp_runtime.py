#!/usr/bin/env python3
"""Verify and extract original ARM64 C++ cache artifacts for inspection only."""
import argparse
import hashlib
import json
import struct
import subprocess
from pathlib import Path

CACHE_SHA256 = "b196c0c60837bb9708605e6ef53a068bc44f1c2872847eb1f1e006e1b9361f84"
CACHE_UUID = bytes.fromhex("7336d75f301433e7843fe1f3522fc52f")
PROVIDERS = ("/usr/lib/libstdc++.6.dylib", "/usr/lib/libc++abi.dylib", "/usr/lib/libSystem.B.dylib")

def digest(path):
    result = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            result.update(chunk)
    return result.hexdigest()

def metadata(path):
    with path.open("rb") as stream:
        header = stream.read(32)
        if len(header) != 32:
            raise ValueError("truncated Mach-O")
        magic, cpu, _, kind, count, size, flags, _ = struct.unpack("<8I", header)
        if (magic, cpu, kind) != (0xFEEDFACF, 0x100000C, 6) or count > 4096 or size > 1024 * 1024:
            raise ValueError("not a bounded ARM64 dylib")
        commands = stream.read(size)
    if len(commands) != size:
        raise ValueError("truncated commands")
    record = {"architecture": "ARM64", "flags": hex(flags), "dependencies": [], "standalone_ready": False}
    offset = 0
    for _ in range(count):
        if offset + 8 > size:
            raise ValueError("truncated command")
        kind, length = struct.unpack_from("<2I", commands, offset)
        if length < 8 or length % 8 or offset + length > size:
            raise ValueError("invalid command length")
        command = commands[offset:offset + length]
        if kind in (0xD, 0xC, 0x80000018, 0x8000001F):
            if length < 24:
                raise ValueError("truncated dylib command")
            name_at, _, current, compat = struct.unpack_from("<4I", command, 8)
            if not 24 <= name_at < length:
                raise ValueError("invalid install name offset")
            end = command.index(0, name_at)
            name = command[name_at:end].decode("utf-8")
            if kind == 0xD:
                record.update(install_name=name, current_version=[current >> 16, current >> 8 & 255, current & 255])
            else:
                record["dependencies"].append({"path": name, "weak": kind == 0x80000018, "reexport": kind == 0x8000001F})
        elif kind in (0x22, 0x80000022):
            if length < 48:
                raise ValueError("truncated dyld info")
            record["rebase_stream_bytes"] = struct.unpack_from("<I", command, 12)[0]
        offset += length
    if offset != size:
        raise ValueError("command size mismatch")
    return record

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cache", type=Path, default=Path("ios-runtime/legacy-cache/System/Library/Caches/com.apple.dyld/dyld_shared_cache_arm64"))
    parser.add_argument("--ipsw", type=Path, default=Path("ios-runtime/tools/ipsw.exe"))
    parser.add_argument("--output", type=Path, default=Path("ios-runtime/legacy-extracted"))
    args = parser.parse_args()
    with args.cache.open("rb") as stream:
        header = stream.read(104)
    if header[88:104] != CACHE_UUID or digest(args.cache) != CACHE_SHA256:
        raise ValueError("original 15G77 cache provenance mismatch")
    args.output.mkdir(parents=True, exist_ok=True)
    (args.output / ".shared-cache-exports").write_text("Original shared-cache exports: audit only, not independently relocatable dylibs. Use coherent original cache.\n", encoding="utf-8")
    files = []
    for provider in PROVIDERS:
        target = args.output / provider.rsplit("/", 1)[1]
        if not target.exists():
            subprocess.run([str(args.ipsw.resolve()), "dyld", "extract", str(args.cache), provider, "--slide", "--output", str(args.output)], check=True)
        info = metadata(target)
        if info.get("install_name") != provider:
            raise ValueError("extracted provider identity differs")
        info.update(file=str(target), size=target.stat().st_size, sha256=digest(target))
        files.append(info)
    if files[0]["current_version"] != [104, 2, 0]:
        raise ValueError("libstdc++ ABI version mismatch")
    report = {"firmware": "iPhone7,2 iOS11.4.1 15G77", "cache": str(args.cache), "cache_sha256": CACHE_SHA256,
              "cache_uuid": CACHE_UUID.hex(), "files": files, "original_cache_preserved": True,
              "runtime_provider": "coherent original cache, including transitive dependencies",
              "standalone_extraction_supported": False,
              "warning": "ipsw exports retain cache-resolved pointers and lack original independent rebases; do not mix them with iOS16 addresses",
              "gameplay_verified": False}
    destination = args.output / "provenance.json"
    destination.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"manifest": str(destination), "providers": len(files), "provenance_verified": True, "standalone_ready": False}))

if __name__ == "__main__":
    main()
