"""Stage the four explicitly authorized legacy test files without launching apps."""
import hashlib
import json
from pathlib import Path
import shlex
import subprocess

ADB = ["adb", "-P", "5038", "-s", "R52Y8066STA"]
PACKAGE = "org.touchhle.android.a64test"
FILES = [
    ("ios-runtime/legacy-cache/System/Library/Caches/com.apple.dyld/dyld_shared_cache_arm64", "ios-runtime/legacy-cache/dyld_shared_cache_arm64", "b196c0c60837bb9708605e6ef53a068bc44f1c2872847eb1f1e006e1b9361f84"),
    ("PokemonQuest-device.ipa", "touchHLE_device_tests/PokemonQuest-device.ipa", "089c73ffa611233a1cb4fa39b2367a829805d06977033e76fdf07cae7d445f6e"),
    ("pixel-fold-tests/arm64-derived/Infinity-Blade-II-v1-3-5-arm64-test.ipa", "touchHLE_device_tests/Infinity-Blade-II-v1-3-5-arm64-test.ipa", "5944afbf8e0cc358ce7d1e619e14c96922f440708aa55ac92443371fefe14aba"),
    ("pixel-fold-tests/arm64-derived/Infinity-Blade-III-v1-4-4-arm64-test.ipa", "touchHLE_device_tests/Infinity-Blade-III-v1-4-4-arm64-test.ipa", "978565968de5b2bfb4fe452f137b016ff856b84483bbcd90e85b7346ba9eb398"),
]

def adb(*args):
    return subprocess.check_output(ADB + list(args), text=True, timeout=900).strip()

def main():
    assert adb("shell", "getprop", "ro.product.device") == "gts11uwifi"
    report = {"serial": "R52Y8066STA", "device": "gts11uwifi", "files": [], "complete": False}
    output = Path("pixel-fold-tests/legacy-tablet-stage.json")
    for source, relative, expected in FILES:
        print("Verifying/staging " + source, flush=True)
        with open(source, "rb") as stream:
            assert hashlib.file_digest(stream, "sha256").hexdigest() == expected
        private = "files/" + relative
        check = subprocess.run(ADB + ["shell", "run-as", PACKAGE, "sha256sum", private], text=True, capture_output=True)
        if check.returncode == 0:
            assert check.stdout.split()[0] == expected, "Existing different file preserved: " + private
            action = "identical-existing"
        else:
            temporary = "/data/local/tmp/playcover-legacy-" + Path(source).name
            adb("push", source, temporary)
            assert adb("shell", "sha256sum", temporary).split()[0] == expected
            part = private + ".verified-staging"
            command = "mkdir -p " + shlex.quote(str(Path(private).parent).replace("\\", "/")) + " && cat > " + shlex.quote(part)
            adb("shell", "cat " + shlex.quote(temporary) + " | run-as " + PACKAGE + " sh -c " + shlex.quote(command))
            assert adb("shell", "run-as", PACKAGE, "sha256sum", part).split()[0] == expected
            adb("shell", "run-as", PACKAGE, "mv", part, private)
            adb("shell", "rm", temporary)
            action = "copied-verified"
        report["files"].append({"source": source, "path": "/data/user/0/" + PACKAGE + "/" + private, "sha256": expected, "bytes": Path(source).stat().st_size, "action": action})
        output.write_text(json.dumps(report, indent=2) + "\n")
    report["complete"] = True
    output.write_text(json.dumps(report, indent=2) + "\n")

if __name__ == "__main__":
    main()
