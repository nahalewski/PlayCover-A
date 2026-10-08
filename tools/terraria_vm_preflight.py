#!/usr/bin/env python3
"""Read-only ordinary ARM64 VM budget audit. Does not map or execute code."""
import argparse
import hashlib
import json
from pathlib import Path
import struct
import zipfile


def segments(raw):
    magic = struct.unpack_from(">I", raw)[0]
    if magic in (0xcafebabe, 0xcafebabf):
        stride = 32 if magic == 0xcafebabf else 20
        count = struct.unpack_from(">I", raw, 4)[0]
        if count > 64 or 8 + count * stride > len(raw):
            raise ValueError("Invalid FAT table")
        for index in range(count):
            base = 8 + index * stride
            if struct.unpack_from(">I", raw, base)[0] == 0x100000c:
                offset, size = struct.unpack_from(">QQ" if stride == 32 else ">II", raw, base + 8)
                if offset + size > len(raw):
                    raise ValueError("Invalid FAT slice")
                raw = raw[offset:offset + size]
                break
        else:
            raise ValueError("No ARM64 slice")
    header = struct.unpack_from("<8I", raw)
    if header[0] != 0xfeedfacf or header[1] != 0x100000c:
        raise ValueError("Expected ARM64 Mach-O")
    count, size = header[4:6]
    end, offset, result = 32 + size, 32, []
    if end > len(raw) or count > 4096:
        raise ValueError("Invalid command table")
    for _ in range(count):
        command, length = struct.unpack_from("<II", raw, offset)
        if length < 8 or offset + length > end:
            raise ValueError("Invalid load command")
        if command == 25:
            if length < 72:
                raise ValueError("Short segment command")
            name, address, virtual, fileoff, filesize, maximum, protection, sections, flags = struct.unpack_from("<16sQQQQIIII", raw, offset + 8)
            if virtual and not (protection == 0 and filesize == 0):
                if address + virtual > 2**64 - 1 or filesize > virtual or fileoff + filesize > len(raw):
                    raise ValueError("Invalid segment range")
                result.append({"name": name.rstrip(b"\0").decode(), "start": address, "end": address + virtual,
                               "bytes": virtual, "protection": protection})
        offset += length
    if offset != end:
        raise ValueError("Command table size mismatch")
    result.sort(key=lambda s: s["start"])
    if any(a["end"] > b["start"] for a, b in zip(result, result[1:])):
        raise ValueError("Overlapping segments within image")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--ipa", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    images = []
    with zipfile.ZipFile(args.ipa) as archive:
        for name in ("Terraria", "Frameworks/UnityFramework.framework/UnityFramework",
                     "Frameworks/AppleCoreNative.framework/AppleCoreNative",
                     "Frameworks/GameKitWrapper.framework/GameKitWrapper",
                     "Frameworks/PlayFabParty.framework/PlayFabParty"):
            data = archive.read("Payload/Terraria.app/" + name)
            ranges = segments(data)
            images.append({"path": name, "sha256": hashlib.sha256(data).hexdigest(),
                           "mapped_bytes": sum(r["bytes"] for r in ranges), "segments": ranges,
                           "within_individual_256_mib_limit": sum(r["bytes"] for r in ranges) <= 256 * 1024**2})
    ordinary = sum(i["mapped_bytes"] for i in images)
    overhead = 0x4000 + 1024**2 + 0x3000
    report = {"scope": "Read-only actual IPA metadata; no mapping, initializer, or game execution.",
              "ipa": str(args.ipa.resolve()), "images": images,
              "ordinary_image_bytes": ordinary, "known_linker_stack_trampoline_resolver_bytes": overhead,
              "remaining_ordinary_256_mib_budget_before_selected_service_memory": 256 * 1024**2 - ordinary - overhead,
              "notes": ["Cache mappings are outside the linker's ordinary 256 MiB budget.",
                        "Selected host-service memory must additionally fit the reported headroom.",
                        "Unslid framework addresses overlap; existing linker relocation is required.",
                        "This report verifies per-image ranges, not final cache/slid mapping addresses."]}
    args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({k: v for k, v in report.items() if k not in ("images", "notes")}, indent=2))


if __name__ == "__main__":
    main()
