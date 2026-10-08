#!/usr/bin/env python3
"""Merge frozen ARM64 diagnostic observations by IPA SHA; no execution implied."""
import argparse
from collections import Counter
import json
from pathlib import Path
import re

from regress_arm64 import fingerprint

RESOLVERS = {
    "1d1affdd8": "libsystem_platform:_OSAtomicCompareAndSwapPtrBarrier",
    "18db91758": "libdispatch:_dispatch_queue_create",
    "18db90510": "libdispatch:_dispatch_after",
    "1d1afff68": "libsystem_platform:_OSAtomicDequeue",
    "18db907b8": "libdispatch:_dispatch_async_f",
    "18db91e10": "libdispatch:_dispatch_sync",
    "18db91960": "libdispatch:_dispatch_retain",
    "18db90bf0": "libdispatch:_dispatch_data_create",
    "18db908f8": "libdispatch:_dispatch_block_cancel",
    "18db90808": "libdispatch:_dispatch_barrier_async",
    "1d1affdb0": "libsystem_platform:_OSAtomicEnqueue",
}
KNOWN_FIXTURES = {"arm64dylibtest.ipa", "arm64initializertest.ipa", "arm64sparsetest.ipa"}


def common_boundary(boundary):
    value = boundary or ""
    for address, name in RESOLVERS.items():
        value = re.sub(r"unaudited cache resolver 0x" + address + r"\b", "unaudited cache resolver " + name, value, flags=re.I)
    value = fingerprint(value)
    value = re.sub(r"\b\d+ TLS (?:initializers|initializer callbacks)\b", "<count> TLS initializer callbacks", value)
    return value


def rows_by_sha(document, field):
    result = {}
    for row in document.get(field, []):
        sha = row.get("sha256")
        if sha:
            result.setdefault(sha, []).append(row)
    return result


def summarize(inventory, baseline, retest=None):
    old = rows_by_sha(baseline, "results")
    retests = retest if isinstance(retest, list) else ([retest] if retest else [])
    new = {}
    for document in retests:
        for sha, observations in rows_by_sha(document, "results").items():
            new.setdefault(sha, []).extend(observations)
    rows, seen = [], set()
    for metadata in inventory.get("ipas", []):
        sha = metadata.get("sha256")
        if sha and sha in seen:
            continue
        if sha:
            seen.add(sha)
        originals, updates = old.get(sha, []), new.get(sha, [])
        latest = (updates or originals or [None])[-1]
        slices = metadata.get("slices", [])
        arm64 = [s for s in slices if s.get("architecture") == "arm64"]
        platform = metadata.get("platform") or ",".join(metadata.get("supported_platforms") or []) or "unknown"
        path = str(metadata.get("path", "")).replace("\\", "/")
        fixture = ("/tests/a64/" in path.lower() or "/fixtures/" in path.lower()
                   or Path(path).name.lower() in KNOWN_FIXTURES)
        television = "tvos" in platform.lower()
        category = "fixture" if fixture else "tvOS package" if television else "iPhone package" if "iphone" in platform.lower() else "unknown platform"
        if latest:
            status = latest.get("status", "unknown")
        elif arm64 and all(s.get("encrypted", False) for s in arm64):
            status = "skipped_encrypted"
        elif not arm64:
            status = "skipped_no_arm64"
        else:
            status = "not_tested"
        rows.append({"sha256": sha, "name": metadata.get("name") or Path(path).stem,
                     "bundle_id": metadata.get("bundle_id"), "path": metadata.get("path"),
                     "architectures": sorted({s.get("architecture", "unknown") for s in slices}),
                     "platform": platform, "sdk": metadata.get("sdk"), "version": metadata.get("version"),
                     "minimum_ios": metadata.get("minimum_ios"), "category": category, "status": status,
                     "boundary": latest.get("boundary") if latest else None,
                     "named_boundary": common_boundary(latest.get("boundary")) if latest and latest.get("boundary") else None,
                     "latest_source": "retest" if updates else "baseline" if originals else None,
                     "baseline": originals, "retest": updates,
                     "inventory": metadata,
                     "app_gameplay_verified": False})
    groups = Counter(common_boundary(r["boundary"]) for r in rows if r["boundary"])
    unmatched = sorted((set(old) | set(new)) - seen)
    return {"schema": 1, "scope": "First diagnostic boundaries only; app main and gameplay are not verified.",
            "unique_inventory_count": len(rows), "rows": rows,
            "common_blockers": [{"boundary": boundary, "count": count} for boundary, count in groups.most_common()],
            "observations_without_inventory_sha": unmatched,
            "baseline_source_stable": baseline.get("frozen_source_verified_after_run"),
            "retest_source_stable": [document.get("frozen_source_verified_after_run") for document in retests]}


def cell(value):
    return str(value if value is not None else "unknown").replace("|", "\\|").replace("\n", " ").replace("\r", " ")


def markdown(report):
    lines = ["# ARM64 batch diagnostic summary", "", report["scope"], "",
             "Metadata comes from the IPA inventory. Latest observations prefer a retest with the same SHA256; raw baseline/retest records are preserved in the adjacent JSON. tvOS packages and fixtures remain distinct categories. A completed diagnostic is not a successful app launch.", "", "## Common first boundaries", ""]
    for group in report["common_blockers"]:
        lines.append(f"- {group['count']}: {cell(group['boundary'])}")
    if not report["common_blockers"]:
        lines.append("No reported first boundary.")
    lines += ["", "## Per-package observations", "", "| App | Architecture | Platform | Version | Minimum OS | Category | Status | Evidence | First boundary |", "|---|---|---|---|---|---|---|---|---|"]
    for row in report["rows"]:
        values = [row["name"], ",".join(row["architectures"]), row["platform"], row["version"], row["minimum_ios"], row["category"], row["status"], row["latest_source"], row["named_boundary"] or "none recorded"]
        lines.append("| " + " | ".join(cell(value) for value in values) + " |")
    return "\n".join(lines) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--inventory", type=Path, required=True)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--retest", type=Path, action="append", default=[], help="Repeat in chronological order; last observation wins")
    parser.add_argument("--output", type=Path, required=True, help="Markdown output; adjacent .json preserves evidence")
    args = parser.parse_args()
    paths = [args.inventory, args.baseline] + args.retest
    if args.output.suffix.lower() != ".md":
        parser.error("--output must end in .md")
    sidecar = args.output.with_suffix(".json")
    if any(p.resolve() in (args.output.resolve(), sidecar.resolve()) for p in paths):
        parser.error("outputs must not overwrite input reports")
    load = lambda path: json.loads(path.read_text(encoding="utf-8"))
    report = summarize(load(args.inventory), load(args.baseline), [load(path) for path in args.retest])
    report["evidence_files"] = {"inventory": str(args.inventory.resolve()), "baseline": str(args.baseline.resolve()), "retest": [str(path.resolve()) for path in args.retest]}
    args.output.write_text(markdown(report), encoding="utf-8")
    sidecar.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"packages": len(report["rows"]), "common_blockers": len(report["common_blockers"]), "markdown": str(args.output), "json": str(sidecar)}))


if __name__ == "__main__":
    main()
