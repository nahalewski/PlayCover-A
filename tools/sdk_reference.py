#!/usr/bin/env python3
"""Resolve SDK reference links from Git modes, without executing SDK contents."""
import argparse
import json
from pathlib import Path
import posixpath
import subprocess


class SDKReference:
    def __init__(self, root, modes):
        self.root = Path(root).resolve()
        self.modes = modes

    @classmethod
    def from_checkout(cls, checkout, sdk="iPhoneOS16.5.sdk"):
        if sdk in ("", ".", "..") or "/" in sdk or "\\" in sdk:
            raise ValueError("SDK name must be one directory")
        result = subprocess.run(["git", "-C", str(checkout), "ls-files", "-s", "-z", "--", sdk], capture_output=True, check=True)
        modes = {}
        for entry in result.stdout.split(b"\0"):
            if not entry:
                continue
            metadata, filename = entry.split(b"\t", 1)
            mode, _, stage = metadata.decode("ascii").split()
            name = filename.decode("utf-8")
            if stage != "0" or not name.startswith(sdk + "/"):
                raise ValueError("unmerged or unexpected Git entry")
            modes[name[len(sdk) + 1:]] = mode
        return cls(Path(checkout) / sdk, modes)

    def resolve(self, relative):
        def normalize(value):
            if "\\" in value or "\0" in value or value.startswith("/") or ":" in value:
                raise ValueError("absolute/invalid SDK path")
            value = posixpath.normpath(value)
            if value == ".." or value.startswith("../"):
                raise ValueError("SDK link escapes root")
            return value

        current = normalize(relative)
        visited = set()
        for _ in range(64):
            if current in visited:
                raise ValueError("SDK link cycle")
            visited.add(current)
            parts = current.split("/")
            changed = False
            for index in range(len(parts)):
                prefix = "/".join(parts[:index + 1])
                path = self.root.joinpath(*parts[:index + 1])
                if self.modes.get(prefix) == "120000":
                    # Git metadata is authoritative; Windows may materialize a
                    # symlink as a plain UTF-8 file containing its relative target.
                    if path.is_symlink():
                        target = str(path.readlink()).replace("\\", "/")
                    else:
                        with path.open("rb") as stream:
                            raw = stream.read(4097)
                        if len(raw) > 4096:
                            raise ValueError("SDK link target too large")
                        target = raw.decode("utf-8")
                    if not target or "\n" in target or "\r" in target:
                        raise ValueError("invalid SDK link target")
                    current = normalize(posixpath.join(posixpath.dirname(prefix), target, *parts[index + 1:]))
                    changed = True
                    break
                if path.is_symlink():
                    raise ValueError("filesystem link absent from Git metadata")
            if changed:
                continue
            if self.modes.get(current) not in ("100644", "100755"):
                raise ValueError("SDK target is not a tracked regular file")
            path = self.root.joinpath(*current.split("/"))
            if not path.is_file() or not path.resolve().is_relative_to(self.root):
                raise ValueError("SDK target missing/outside root")
            return path
        raise ValueError("SDK link depth exceeds 64")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--checkout", type=Path, default=Path(__file__).resolve().parents[1] / "ios-runtime/sdk/theos-reference")
    parser.add_argument("--sdk", default="iPhoneOS16.5.sdk")
    parser.add_argument("path", help="Path inside the SDK")
    args = parser.parse_args()
    reference = SDKReference.from_checkout(args.checkout, args.sdk)
    result = reference.resolve(args.path)
    print(json.dumps({"requested": args.path, "resolved": str(result), "sdk_root": str(reference.root)}))


if __name__ == "__main__":
    main()
