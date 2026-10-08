#!/usr/bin/env python3
"""Bounded, offline IPA metadata inventory; never extracts or executes binaries."""
import argparse
import hashlib
import json
from pathlib import Path
import plistlib
import struct
import sys
import zipfile

MAX_COMMANDS = 1024 * 1024


def read_at(stream, offset, size, limit):
    if offset < 0 or size < 0 or offset + size > limit:
        raise ValueError("Mach-O range outside executable")
    stream.seek(offset)
    result = stream.read(size)
    if len(result) != size:
        raise ValueError("truncated executable")
    return result


def thin(stream, offset, size, limit):
    magic = read_at(stream, offset, 4, limit)
    formats = {b"\xce\xfa\xed\xfe": ("<", False),
               b"\xcf\xfa\xed\xfe": ("<", True),
               b"\xfe\xed\xfa\xce": (">", False),
               b"\xfe\xed\xfa\xcf": (">", True)}
    if magic not in formats:
        raise ValueError("unsupported Mach-O magic")
    endian, wide = formats[magic]
    header_size = 32 if wide else 28
    if size < header_size:
        raise ValueError("short Mach-O slice")
    header = read_at(stream, offset, header_size, limit)
    cpu, subtype, file_type, count, commands_size = struct.unpack_from(endian + "IIIII", header, 4)
    if count > 65536 or commands_size > MAX_COMMANDS or header_size + commands_size > size:
        raise ValueError("invalid or excessive load commands")
    commands = read_at(stream, offset + header_size, commands_size, limit)
    cursor, encryption = 0, []
    for _ in range(count):
        if cursor + 8 > len(commands):
            raise ValueError("truncated load command")
        cmd, length = struct.unpack_from(endian + "II", commands, cursor)
        if length < 8 or length % 4 or cursor + length > len(commands):
            raise ValueError("invalid load command size")
        if cmd in (0x21, 0x2C):
            if length < (24 if cmd == 0x2C else 20):
                raise ValueError("short encryption command")
            start, amount, cryptid = struct.unpack_from(endian + "III", commands, cursor + 8)
            if start + amount > size:
                raise ValueError("encryption range outside slice")
            encryption.append({"cryptid": cryptid, "offset": start, "size": amount})
        cursor += length
    if cursor != len(commands):
        raise ValueError("load command size mismatch")
    architecture = {12: "arm32", 0x100000C: "arm64", 0x200000C: "arm64_32",
                    7: "x86", 0x1000007: "x86_64"}.get(cpu, "unknown")
    return {"architecture": architecture, "cpu_type": cpu, "cpu_subtype": subtype,
            "file_type": file_type, "offset": offset, "size": size,
            "encryption": encryption, "encrypted": any(e["cryptid"] != 0 for e in encryption)}


def macho(stream, size):
    magic = read_at(stream, 0, 4, size)
    fat = {b"\xca\xfe\xba\xbe": (">", False), b"\xbe\xba\xfe\xca": ("<", False),
           b"\xca\xfe\xba\xbf": (">", True), b"\xbf\xba\xfe\xca": ("<", True)}
    if magic not in fat:
        return [thin(stream, 0, size, size)]
    endian, wide = fat[magic]
    count = struct.unpack(endian + "I", read_at(stream, 4, 4, size))[0]
    if not 1 <= count <= 64:
        raise ValueError("invalid FAT slice count")
    stride = 32 if wide else 20
    table_end = 8 + stride * count
    table = read_at(stream, 8, stride * count, size)
    result, intervals = [], []
    for index in range(count):
        fmt = endian + ("IIQQII" if wide else "IIIII")
        fields = struct.unpack_from(fmt, table, index * stride)
        cpu, subtype, offset, amount, alignment = fields[:5]
        if alignment > 63 or offset % (1 << alignment) or offset < table_end or amount == 0 or offset + amount > size:
            raise ValueError("invalid FAT slice range/alignment")
        if any(offset < end and start < offset + amount for start, end in intervals):
            raise ValueError("overlapping FAT slices")
        intervals.append((offset, offset + amount))
        item = thin(stream, offset, amount, size)
        if item["cpu_type"] != cpu or item["cpu_subtype"] != subtype:
            raise ValueError("FAT/header architecture mismatch")
        result.append(item)
    return result


def inspect(path, progress=False):
    result = {"path": str(path.resolve())}
    try:
        if progress:
            print(f"Hashing {path}", file=sys.stderr, flush=True)
        digest = hashlib.sha256()
        with path.open("rb") as source:
            for block in iter(lambda: source.read(1024 * 1024), b""):
                digest.update(block)
        result.update(sha256=digest.hexdigest(), size=path.stat().st_size)
        if progress:
            print(f"Reading metadata {path.name}", file=sys.stderr, flush=True)
        with zipfile.ZipFile(path) as archive:
            infos = archive.infolist()
            candidates = [i for i in infos if i.filename.startswith("Payload/")
                          and i.filename.count("/") == 2 and i.filename.endswith(".app/Info.plist")]
            if len(candidates) != 1:
                raise ValueError("expected exactly one top-level application Info.plist")
            info = candidates[0]
            if info.file_size > 4 * 1024 * 1024:
                raise ValueError("Info.plist too large")
            metadata = plistlib.loads(archive.read(info))
            if not isinstance(metadata, dict):
                raise ValueError("Info.plist root is not a dictionary")
            executable = metadata.get("CFBundleExecutable")
            if not isinstance(executable, str) or not executable or "/" in executable or "\\" in executable or executable in (".", ".."):
                raise ValueError("invalid CFBundleExecutable")
            name = info.filename.rsplit("/", 1)[0] + "/" + executable
            matches = [i for i in infos if i.filename == name]
            if len(matches) != 1:
                raise ValueError("missing or duplicate executable")
            binary = matches[0]
            with archive.open(binary) as stream:
                slices = macho(stream, binary.file_size)
            app_name = next((value.strip() for value in
                             (metadata.get("CFBundleDisplayName"), metadata.get("CFBundleName"))
                             if isinstance(value, str) and value.strip()), path.stem)
            result.update(name=app_name,
                          bundle_id=metadata.get("CFBundleIdentifier"),
                          version=metadata.get("CFBundleShortVersionString"),
                          minimum_ios=metadata.get("MinimumOSVersion"), executable=name,
                          platform=metadata.get("DTPlatformName"), sdk=metadata.get("DTSDKName"),
                          device_family=metadata.get("UIDeviceFamily"),
                          supported_platforms=metadata.get("CFBundleSupportedPlatforms"),
                          slices=slices, has_arm64=any(s["architecture"] == "arm64" for s in slices))
    except (OSError, ValueError, TypeError, KeyError, EOFError, struct.error, zipfile.BadZipFile, RuntimeError, NotImplementedError) as error:
        result["error"] = str(error)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--ipa-dir", type=Path, default=Path.home() / "Desktop" / "ipa", help="Only this directory is searched recursively")
    parser.add_argument("--downloads", type=Path, default=Path.home() / "Downloads")
    parser.add_argument("--workspace", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--output", type=Path, help="JSON output file; stdout if omitted")
    parser.add_argument("--checkpoint", action="store_true", help="Atomically update --output after every IPA")
    args = parser.parse_args()
    if args.checkpoint and not args.output:
        parser.error("--checkpoint requires --output")
    paths = set()
    for directory, recursive in ((args.ipa_dir, True), (args.downloads, False), (args.workspace, False)):
        if directory.is_dir():
            for path in (directory.rglob("*") if recursive else directory.iterdir()):
                if path.is_file() and path.suffix.lower() == ".ipa":
                    paths.add(path.resolve())
    records, first = [], {}
    def publish(complete):
        output = json.dumps({"schema": 1, "complete": complete, "discovered_count": len(paths), "ipa_count": len(records), "unique_sha256_count": len(first), "ipas": records}, indent=2, default=str)
        if args.output:
            temporary = args.output.with_name(args.output.name + ".tmp")
            temporary.write_text(output + "\n", encoding="utf-8")
            temporary.replace(args.output)
        return output

    for index, path in enumerate(sorted(paths, key=lambda p: str(p).lower()), 1):
        print(f"[{index}/{len(paths)}] {path.name}", file=sys.stderr, flush=True)
        record = inspect(path, progress=True)
        digest = record.get("sha256")
        if digest in first:
            record["duplicate_of"] = first[digest]
        elif digest:
            first[digest] = record["path"]
        records.append(record)
        if args.checkpoint:
            publish(False)
    output = publish(True)
    if not args.output:
        print(output)


if __name__ == "__main__":
    main()
