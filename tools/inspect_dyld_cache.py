#!/usr/bin/env python3
"""Offline, bounded inspection of an arm64 dyld cache and its companions.

Reads headers and metadata only. Does not execute code or modify cache files.
Layout source: Apple's include/mach-o/dyld_cache_format.h (dyld repository).
"""
import argparse
import bisect
import json
from pathlib import Path
import re
import struct
import tempfile
import unittest
import uuid

SOURCE = "https://github.com/apple-oss-distributions/dyld/blob/main/include/mach-o/dyld_cache_format.h"
U64_MAX = (1 << 64) - 1
MAX_METADATA_READ = 64 * 1024 * 1024
MAX_TABLE_ENTRIES = 100_000
MAX_SLIDE_PAGES = 1_000_000


def require(condition, message):
    if not condition:
        raise ValueError(message)


def extent(offset, size, limit, label):
    require(0 <= offset <= limit and 0 <= size <= limit - offset,
            f"{label} outside bounds: offset={offset:#x}, size={size:#x}, limit={limit:#x}")


def unpack(data, fmt, offset=0):
    extent(offset, struct.calcsize(fmt), len(data), "metadata field")
    return struct.unpack_from(fmt, data, offset)


def uuid_string(value):
    return str(uuid.UUID(bytes=value))


class CacheFile:
    def __init__(self, path):
        self.path = Path(path)
        self.size = self.path.stat().st_size
        require(self.size >= 0x200, f"truncated cache header: {self.path}")
        self.header = self.read(0, 0x200)
        require(self.header[:16] == b"dyld_v1   arm64\0", f"unsupported cache magic: {self.path}")
        self.uuid = self.header[0x58:0x68]
        self.mapping_offset, self.mapping_count = unpack(self.header, "<II", 0x10)
        require(self.mapping_offset >= 0x1C8, "requires modern cache header with image/subcache fields")
        extent(0, self.mapping_offset, self.size, "cache header extent")
        self.region_start, self.region_size, self.max_slide = unpack(self.header, "<QQQ", 0xE0)

    def read(self, offset, size):
        extent(offset, size, self.size, str(self.path.name))
        require(size <= MAX_METADATA_READ, "metadata read exceeds loader inspection limit")
        with self.path.open("rb") as stream:
            stream.seek(offset)
            result = stream.read(size)
        require(len(result) == size, "cache changed or truncated while reading")
        return result

    def table(self, offset, count, stride):
        require(count <= MAX_TABLE_ENTRIES, "table count exceeds loader inspection limit")
        require(count == 0 or offset >= self.mapping_offset, "table overlaps cache header")
        return self.read(offset, count * stride)

    def cstring(self, offset, maximum=4096):
        extent(offset, 1, self.size, "cache image path")
        data = self.read(offset, min(maximum + 1, self.size - offset))
        end = data.find(b"\0")
        require(0 < end <= maximum, "empty, unterminated or oversized cache image path")
        return data[:end].decode("utf-8", "strict")


def slide_v2(data, mapping_size):
    version, page_size, starts_offset, starts_count, extras_offset, extras_count, mask, value_add = unpack(data, "<IIIIIIQQ")
    require(version == 2, "not slide info version 2")
    require(starts_count <= MAX_SLIDE_PAGES and extras_count <= MAX_SLIDE_PAGES,
            "slide v2 table exceeds inspection limit")
    require(page_size in (4096, 16384), "unsupported slide v2 page size")
    require(mapping_size % page_size == 0 and starts_count == mapping_size // page_size,
            "slide v2 page count does not match mapping")
    require(mask != 0, "empty slide v2 delta mask")
    trailing_zeros = (mask & -mask).bit_length() - 1
    shifted = mask >> trailing_zeros
    require(trailing_zeros >= 2 and shifted & (shifted + 1) == 0,
            "noncontiguous or unaligned slide v2 delta mask")
    require(starts_offset >= 40 and (extras_count == 0 or extras_offset >= 40),
            "slide v2 tables overlap header")
    extent(starts_offset, starts_count * 2, len(data), "slide v2 starts")
    extent(extras_offset, extras_count * 2, len(data), "slide v2 extras")
    if extras_count:
        require(starts_offset + starts_count * 2 <= extras_offset or extras_offset + extras_count * 2 <= starts_offset,
                "slide v2 tables overlap")
    starts = unpack(data, f"<{starts_count}H", starts_offset) if starts_count else ()
    extras = unpack(data, f"<{extras_count}H", extras_offset) if extras_count else ()
    empty_pages = extra_pages = chains = 0
    # Compute suffix-chain lengths backwards once: repeated or overlapping
    # extras lists must not amplify a small table into quadratic work.
    extra_summaries = [None] * len(extras)
    for index in range(len(extras) - 1, -1, -1):
        extra = extras[index]
        if extra & 0x4000 or (extra & 0x3FFF) * 4 + 8 > page_size:
            continue
        if extra & 0x8000:
            extra_summaries[index] = 1
        elif index + 1 < len(extras) and extra_summaries[index + 1] is not None:
            extra_summaries[index] = extra_summaries[index + 1] + 1
    for start in starts:
        if start == 0x4000:
            empty_pages += 1
        elif start & 0x8000:
            require(start & 0x4000 == 0, "invalid slide v2 page flags")
            extra_pages += 1
            index = start & 0x3FFF
            require(index < len(extras) and extra_summaries[index] is not None,
                    "unterminated, out-of-page or invalid slide v2 extras index")
            chains += extra_summaries[index]
        else:
            require(start & 0x4000 == 0 and start * 4 + 8 <= page_size,
                    "slide v2 chain start outside page")
            chains += 1
    return dict(version=version, page_size=page_size, page_starts_offset=starts_offset,
                page_starts_count=starts_count, page_extras_offset=extras_offset,
                page_extras_count=extras_count, delta_mask=mask,
                delta_mask_hex=hex(mask), delta_shift=trailing_zeros - 2,
                value_add=value_add, value_add_hex=hex(value_add),
                pages_without_rebases=empty_pages, pages_with_extra_starts=extra_pages,
                chain_start_count=chains, pointer_chains_validated=False)


def mappings(cache):
    original = cache.table(cache.mapping_offset, cache.mapping_count, 32)
    old = [unpack(original, "<QQQII", i * 32) for i in range(cache.mapping_count)]
    offset, count = unpack(cache.header, "<II", 0x138)
    require(count == cache.mapping_count, "mapping and mapping-with-slide counts differ")
    records = cache.table(offset, count, 56)
    result = []
    for i in range(count):
        address, size, file_offset, slide_offset, slide_size, flags, max_prot, init_prot = unpack(records, "<QQQQQQII", i * 56)
        require((address, size, file_offset, max_prot, init_prot) == old[i], "mapping tables disagree")
        require(size > 0 and address + size <= U64_MAX, "invalid mapping virtual range")
        require(max_prot & ~7 == 0 and init_prot & ~max_prot == 0, "invalid mapping protections")
        require(address % 4096 == file_offset % 4096 == size % 4096 == 0, "unaligned mapping")
        extent(file_offset, size, cache.size, "mapping file range")
        row = dict(file=cache.path.name, address=address, address_hex=hex(address),
                   end_address=address + size, end_address_hex=hex(address + size), size=size,
                   file_offset=file_offset, flags=flags, max_prot=max_prot, init_prot=init_prot,
                   slide_info_file_offset=slide_offset, slide_info_file_size=slide_size,
                   slide_info=None)
        if slide_size:
            data = cache.read(slide_offset, slide_size)
            version, = unpack(data, "<I")
            require(version == 2, f"unsupported slide info version {version} in {cache.path.name}")
            row["slide_info"] = slide_v2(data, size)
        else:
            require(slide_offset == 0, "empty slide info has nonzero offset")
        result.append(row)
    return result


def inspect(main_path):
    main = CacheFile(main_path)
    base = main.region_start
    require(base + main.region_size <= U64_MAX, "shared region overflow")
    sub_offset, sub_count = unpack(main.header, "<II", 0x188)
    entries = main.table(sub_offset, sub_count, 56)
    files = [main]
    file_rows = [dict(file=main.path.name, size=main.size, uuid=uuid_string(main.uuid), role="main")]
    all_mappings = mappings(main)
    suffixes = set()
    for i in range(sub_count):
        entry = entries[i * 56:(i + 1) * 56]
        expected_uuid = entry[:16]
        vm_offset, = unpack(entry, "<Q", 16)
        raw_suffix = entry[24:]
        require(b"\0" in raw_suffix, "unterminated companion suffix")
        suffix = raw_suffix.split(b"\0", 1)[0].decode("ascii")
        require(re.fullmatch(r"\.[A-Za-z0-9_.]+", suffix) and ".." not in suffix,
                "unsafe companion suffix")
        require(suffix not in suffixes, "duplicate companion suffix")
        suffixes.add(suffix)
        companion = CacheFile(main.path.with_name(main.path.name + suffix))
        require(companion.uuid == expected_uuid, f"companion UUID mismatch: {companion.path.name}")
        # Subcache headers record their own mapping base and commonly zero the
        # overall shared-region size; the main header owns that region contract.
        require((companion.region_size == 0 and companion.region_start == base + vm_offset)
                or (companion.region_start == base and companion.region_size == main.region_size),
                "companion shared region does not match main cache")
        regions = mappings(companion)
        require(regions and regions[0]["address"] == base + vm_offset, "companion VM offset mismatch")
        files.append(companion)
        file_rows.append(dict(file=companion.path.name, size=companion.size,
                              uuid=uuid_string(companion.uuid), role="subcache", cache_vm_offset=vm_offset))
        all_mappings.extend(regions)
    symbol_uuid = main.header[0x190:0x1A0]
    if symbol_uuid != bytes(16):
        symbol = CacheFile(main.path.with_name(main.path.name + ".symbols"))
        require(symbol.uuid == symbol_uuid, "symbols companion UUID mismatch")
        local_offset, local_size = unpack(symbol.header, "<QQ", 0x48)
        extent(local_offset, local_size, symbol.size, "local symbols")
        file_rows.append(dict(file=symbol.path.name, size=symbol.size,
                              uuid=uuid_string(symbol.uuid), role="unmapped_symbols"))
    all_mappings.sort(key=lambda m: m["address"])
    for index, row in enumerate(all_mappings):
        require(base <= row["address"] and row["end_address"] <= base + main.region_size,
                "mapping outside declared shared region")
        if index:
            require(all_mappings[index - 1]["end_address"] <= row["address"], "overlapping VM mappings")
    addresses = [m["address"] for m in all_mappings]
    image_offset, image_count = unpack(main.header, "<II", 0x1C0)
    image_table = main.table(image_offset, image_count, 32)
    images = []
    for i in range(image_count):
        address, _, _, path_offset, _ = unpack(image_table, "<QQQII", i * 32)
        index = bisect.bisect_right(addresses, address) - 1
        require(index >= 0 and address + 32 <= all_mappings[index]["end_address"], "image header is unmapped")
        mapping = all_mappings[index]
        image_file_offset = mapping["file_offset"] + address - mapping["address"]
        cache_file = next(f for f in files if f.path.name == mapping["file"])
        magic, cpu_type = unpack(cache_file.read(image_file_offset, 8), "<II")
        require(magic == 0xFEEDFACF and cpu_type == 0x100000C, "cache image is not arm64 Mach-O")
        images.append(dict(path=main.cstring(path_offset), address=address, address_hex=hex(address),
                           header_file=mapping["file"], header_file_offset=image_file_offset))
    mapped_bytes = sum(m["size"] for m in all_mappings)
    span = all_mappings[-1]["end_address"] - all_mappings[0]["address"] if all_mappings else 0
    return dict(schema_version=1, source_format=SOURCE, cache_root=str(main.path.parent.resolve()),
                main_file=main.path.name, shared_region_start=base, shared_region_size=main.region_size,
                maximum_slide=main.max_slide, mapped_bytes=mapped_bytes, mapped_span_bytes=span,
                holes_bytes=span - mapped_bytes, file_count=len(file_rows), image_count=len(images),
                validations=["arm64 cache magic", "companion UUIDs", "mapping bounds and protections",
                             "nonoverlapping VM intervals", "slide v2 metadata tables and start locations",
                             "image paths and mapped arm64 headers"],
                remaining_work=["sparse guest mapping rather than a flat cache-sized allocation",
                                "bounded slide v2 pointer-chain decoding before execution",
                                "image exports/reexports and app-to-cache symbol binding",
                                "Darwin kernel, TLS, initializers and Objective-C runtime integration"],
                pointer_chains_validated=False, execution_verified=False, files=file_rows,
                intervals=all_mappings, images=images)


class InspectorTests(unittest.TestCase):
    def cache_fixture(self, path, base, identity, image=False):
        data = bytearray(4096)
        data[:16] = b"dyld_v1   arm64\0"
        data[0x58:0x68] = identity
        struct.pack_into("<II", data, 0x10, 512, 1)
        struct.pack_into("<QQQ", data, 0xE0, base, 65536 if image else 0, 0)
        struct.pack_into("<II", data, 0x138, 544, 1)
        struct.pack_into("<QQQII", data, 512, base, 4096, 0, 5, 5)
        struct.pack_into("<QQQQQQII", data, 544, base, 4096, 0, 0, 0, 0, 5, 5)
        if image:
            struct.pack_into("<II", data, 0x1C0, 624, 1)
            struct.pack_into("<QQQII", data, 624, base + 1024, 0, 0, 656, 0)
            data[656:676] = b"/usr/lib/test.dylib\0\0"
            struct.pack_into("<II", data, 1024, 0xFEEDFACF, 0x100000C)
        path.write_bytes(data)

    def test_companion_uuid_and_mapping_bounds(self):
        with tempfile.TemporaryDirectory() as directory:
            main_path = Path(directory) / "dyld_shared_cache_arm64"
            companion_path = Path(directory) / "dyld_shared_cache_arm64.01"
            base = 0x180000000
            self.cache_fixture(main_path, base, bytes(16), image=True)
            self.cache_fixture(companion_path, base + 4096, bytes([1]) * 16)
            data = bytearray(main_path.read_bytes())
            struct.pack_into("<II", data, 0x188, 704, 1)
            data[704:720] = bytes([1]) * 16
            struct.pack_into("<Q", data, 720, 4096)
            data[728:732] = b".01\0"
            main_path.write_bytes(data)
            report = inspect(main_path)
            self.assertEqual(report["image_count"], 1)
            self.assertEqual(report["mapped_bytes"], 8192)
            data = bytearray(companion_path.read_bytes())
            data[0x58] = 2
            companion_path.write_bytes(data)
            with self.assertRaisesRegex(ValueError, "UUID mismatch"):
                inspect(main_path)
            data[0x58] = 1
            struct.pack_into("<Q", data, 512 + 16, 4096)
            struct.pack_into("<Q", data, 544 + 16, 4096)
            companion_path.write_bytes(data)
            with self.assertRaisesRegex(ValueError, "mapping file range"):
                inspect(main_path)

    def slide_fixture(self, start=1):
        return struct.pack("<IIIIIIQQH", 2, 4096, 40, 1, 42, 0, 0x3FF0000000000000, 0x180000000, start)

    def test_slide_metadata(self):
        report = slide_v2(self.slide_fixture(), 4096)
        self.assertEqual(report["chain_start_count"], 1)
        self.assertEqual(report["delta_shift"], 50)
        self.assertEqual(slide_v2(self.slide_fixture(0x4000), 4096)["pages_without_rebases"], 1)

    def test_rejects_bounds_flags_and_truncation(self):
        for data in (self.slide_fixture()[:-1], self.slide_fixture(0x3FFF), self.slide_fixture(0x8000)):
            with self.assertRaises(ValueError):
                slide_v2(data, 4096)
        with self.assertRaises(ValueError):
            extent(10, 2, 11, "test")

    def test_extra_starts(self):
        data = struct.pack("<IIIIIIQQHHH", 2, 4096, 40, 1, 42, 2, 0x3FF0000000000000, 0x180000000,
                           0x8000, 1, 0x8004)
        self.assertEqual(slide_v2(data, 4096)["chain_start_count"], 2)

    def test_bad_magic(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "cache"
            path.write_bytes(bytes(512))
            with self.assertRaises(ValueError):
                CacheFile(path)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cache", nargs="?", type=Path,
                        default=Path("ios-runtime/cache/System/Library/Caches/com.apple.dyld/dyld_shared_cache_arm64"))
    parser.add_argument("--output", type=Path, default=Path("ios-runtime/cache-loader-plan.json"))
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(InspectorTests)
        raise SystemExit(not unittest.TextTestRunner().run(suite).wasSuccessful())
    try:
        report = inspect(args.cache)
    except (OSError, ValueError, UnicodeError, struct.error) as error:
        parser.exit(1, f"cache inspection failed: {error}\n")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({key: report[key] for key in ("file_count", "image_count", "mapped_bytes", "mapped_span_bytes", "holes_bytes")}))
    print(f"Wrote {args.output}")


if __name__ == "__main__":
    main()
