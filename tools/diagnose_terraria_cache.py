#!/usr/bin/env python3
"""Reproduce Terraria cache binding diagnostics; never request app execution.

Run in WSL with the existing desktop binary and original iOS16 shared cache.
The actual IPA is tested first. Four temporary executable scaffolds then load
each unchanged embedded framework, including dynamically loaded UnityFramework.
Original source files are read only. Reports explicitly distinguish scaffolds.
"""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import plistlib
import shutil
import struct
import subprocess
import tempfile
import zipfile


def digest(path):
    value = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(block)
    return value.hexdigest()


def scaffold(provider):
    def string_command(command, text, header_size):
        raw = text.encode() + b"\0"
        size = (header_size + len(raw) + 7) & ~7
        return (struct.pack("<III", command, size, header_size)
                + bytes(header_size - 12) + raw + bytes(size - header_size - len(raw)))
    segment = struct.pack("<II16sQQQQIIII", 25, 72, b"__TEXT",
                          0x100000000, 4096, 0, 4096, 5, 5, 0, 0)
    commands = [segment,
                string_command(0x8000001c, "@executable_path/Frameworks", 12),
                string_command(12, f"@rpath/{provider}.framework/{provider}", 24),
                struct.pack("<IIQQ", 0x80000028, 24, 0x800, 0)]
    raw = (struct.pack("<8I", 0xfeedfacf, 0x100000c, 0, 2, len(commands),
                       sum(map(len, commands)), 0x200085, 0) + b"".join(commands)).ljust(4096, b"\0")
    return raw[:0x800] + struct.pack("<I", 0xd65f03c0) + raw[0x804:]


def main():
    workspace = Path(__file__).resolve().parent.parent
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--ipa", type=Path, default=Path("/mnt/c/Users/Ben/Desktop/ipa/Terraria_4.5.0.ipa"))
    parser.add_argument("--binary", type=Path, default=Path("/home/ben/touchHLE-a64/target/debug/touchHLE"))
    parser.add_argument("--cache", type=Path, default=workspace / "ios-runtime/cache/System/Library/Caches/com.apple.dyld/dyld_shared_cache_arm64")
    parser.add_argument("--output-dir", type=Path, default=workspace / "pixel-fold-tests")
    parser.add_argument("--timeout", type=int, default=180)
    parser.add_argument("--only-root", choices=["Main", "UnityFramework", "AppleCoreNative", "GameKitWrapper", "PlayFabParty"])
    args = parser.parse_args()
    args.ipa = args.ipa.resolve(strict=True)
    args.binary = args.binary.resolve(strict=True)
    args.cache = args.cache.resolve(strict=True)
    args.output_dir.mkdir(parents=True, exist_ok=True)
    with args.cache.open("rb") as stream:
        header = stream.read(104)
    if header[:16] != b"dyld_v1   arm64\0":
        raise ValueError("Expected original ARM64 shared cache")
    base = {
        "tested_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "source": str(args.ipa), "ipa_sha256": digest(args.ipa),
        "binary": str(args.binary), "binary_sha256": digest(args.binary),
        "cache": str(args.cache), "cache_header_uuid": header[88:104].hex(),
        "cache_main_sha256": digest(args.cache),
        "app_functionality_verified": False, "app_code_executed": False,
    }
    summaries = []

    def run(app, mode, label, extra=None):
        command = [str(args.binary), str(app), "--no-error-popup", f"--{mode}={args.cache}"]
        env = dict(os.environ, SDL_VIDEODRIVER="dummy", SDL_AUDIODRIVER="dummy")
        try:
            process = subprocess.run(command, capture_output=True, text=True, cwd=workspace,
                                     env=env, timeout=args.timeout)
            code, log = process.returncode, process.stdout + process.stderr
        except subprocess.TimeoutExpired as error:
            def decoded(value):
                return value.decode(errors="replace") if isinstance(value, bytes) else (value or "")
            code, log = None, decoded(error.stdout) + decoded(error.stderr)
            log += "\nDiagnostic timed out; no success inferred.\n"
        report = dict(base, label=label, command=command, returncode=code, log=log,
                      scope="Desktop metadata and binding diagnostics only; no initializers/main executed.")
        report.update(extra or {})
        filename = f"terraria-{label}-desktop.json"
        (args.output_dir / filename).write_text(json.dumps(report, indent=2) + "\n")
        blocker = next((line for line in log.splitlines() if line.startswith("Error:")), None)
        summaries.append({"report": filename, "root": report.get("explicit_provider", "Terraria actual IPA"),
                          "mode": mode, "returncode": code, "first_blocker": blocker,
                          "import_missing_count": None})
        print(json.dumps({"report": filename, "returncode": code, "first_blocker": blocker}), flush=True)

    if args.only_root in (None, "Main"):
        run(args.ipa, "a64-cache-import-test", "ios16-import")
        run(args.ipa, "a64-cache-prepare", "ios16-prepare")
    with tempfile.TemporaryDirectory(prefix="terraria-cache-audit-") as temporary:
        root = Path(temporary)
        with zipfile.ZipFile(args.ipa) as archive:
            for item in archive.infolist():
                if not item.filename.startswith("Payload/Terraria.app/"):
                    continue
                relative = PurePosixPath(item.filename)
                if relative.is_absolute() or ".." in relative.parts:
                    raise ValueError("Unsafe ZIP member")
                destination = root.joinpath(*relative.parts)
                if item.is_dir():
                    destination.mkdir(parents=True, exist_ok=True)
                else:
                    destination.parent.mkdir(parents=True, exist_ok=True)
                    with archive.open(item) as source, destination.open("wb") as target:
                        shutil.copyfileobj(source, target)
        app = root / "Payload/Terraria.app"
        info_path = app / "Info.plist"
        info = plistlib.loads(info_path.read_bytes())
        for provider in ["UnityFramework", "AppleCoreNative", "GameKitWrapper", "PlayFabParty"]:
            if args.only_root is not None and provider != args.only_root:
                continue
            original = app / f"Frameworks/{provider}.framework/{provider}"
            info["CFBundleExecutable"] = "AuditScaffold"
            info_path.write_bytes(plistlib.dumps(info))
            (app / "AuditScaffold").write_bytes(scaffold(provider))
            run(app, "a64-cache-prepare", provider.lower() + "-ios16-scaffold-prepare", {
                "scope": "Temporary synthetic MH_EXECUTE load scaffold with unchanged original embedded provider and full copied bundle; no initializers/main executed.",
                "explicit_provider": provider, "provider_sha256": digest(original),
                "scaffold_sha256": digest(app / "AuditScaffold"),
            })
    summary = dict(base, roots=summaries,
                   note="Unavailable classic-import counts remain null, never zero. Scaffold failures are binding diagnostics, not app launch results.")
    summary_name = f"terraria-{args.only_root.lower()}-ios16-single-root-summary.json" if args.only_root else "terraria-ios16-desktop-summary.json"
    (args.output_dir / summary_name).write_text(json.dumps(summary, indent=2) + "\n")


if __name__ == "__main__":
    main()
