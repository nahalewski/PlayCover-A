"""Exercise the real IPA launch path with a freestanding ARM64 payload."""
import pathlib
import plistlib
import subprocess
import tempfile
import zipfile
import sys

binary, payload = map(pathlib.Path, sys.argv[1:3])
with tempfile.TemporaryDirectory(prefix="playcover-a64-") as folder:
    folder = pathlib.Path(folder)
    ipa = folder / "Arm64Hello.ipa"
    info = {
        "CFBundleIdentifier": "local.playcover.arm64hello",
        "CFBundleName": "Arm64Hello",
        "CFBundleExecutable": "Arm64Hello",
        "CFBundleVersion": "1",
        "MinimumOSVersion": "6.0",
    }
    with zipfile.ZipFile(ipa, "w", zipfile.ZIP_DEFLATED) as archive:
        archive.writestr("Payload/Arm64Hello.app/Info.plist", plistlib.dumps(info))
        archive.write(payload, "Payload/Arm64Hello.app/Arm64Hello")
    result = subprocess.run(
        [str(binary), str(ipa), "--headless"], cwd=folder,
        capture_output=True, text=True, timeout=30,
    )
    output = result.stdout + result.stderr
    assert result.returncode == 0, output
    assert "Hello from arm64 Mach-O on dynarmic A64!" in output, output
    assert "ARM64 guest exited with code 186" in output, output
    print("ARM64 IPA launch path passed")
