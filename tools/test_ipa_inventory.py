import io
import plistlib
from pathlib import Path
import struct
import tempfile
import unittest
import zipfile

import ipa_inventory as inventory


def binary(encrypted=False):
    command = struct.pack("<IIIIII", 0x2C, 24, 0, 0, 1, 0) if encrypted else b""
    return struct.pack("<IIIIIIII", 0xFEEDFACF, 0x100000C, 0, 2, int(encrypted), len(command), 0, 0) + command


class InventoryTests(unittest.TestCase):
    def test_thin_encryption(self):
        data = binary(True)
        result = inventory.macho(io.BytesIO(data), len(data))[0]
        self.assertEqual(result["architecture"], "arm64")
        self.assertTrue(result["encrypted"])
        self.assertEqual(result["encryption"][0]["cryptid"], 1)

    def test_fat_and_bounds(self):
        data = binary()
        fat = struct.pack(">II", 0xCAFEBABE, 1) + struct.pack(">IIIII", 0x100000C, 0, 32, len(data), 4) + bytes(4) + data
        self.assertEqual(inventory.macho(io.BytesIO(fat), len(fat))[0]["offset"], 32)
        malformed = bytearray(fat)
        struct.pack_into(">I", malformed, 16, 16)
        with self.assertRaises(ValueError):
            inventory.macho(io.BytesIO(malformed), len(malformed))

    def test_load_command_bounds(self):
        data = bytearray(binary(True))
        struct.pack_into("<I", data, 36, 0x1000)
        with self.assertRaises(ValueError):
            inventory.macho(io.BytesIO(data), len(data))

    def test_real_zip_and_corrupt_zip(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "test.ipa"
            with zipfile.ZipFile(path, "w") as archive:
                archive.writestr("Payload/Test.app/Info.plist", plistlib.dumps({"CFBundleExecutable": "Test", "CFBundleIdentifier": "test.example", "DTPlatformName": "appletvos", "DTSDKName": "appletvos17.2", "UIDeviceFamily": [3], "CFBundleSupportedPlatforms": ["AppleTVOS"]}))
                archive.writestr("Payload/Test.app/Test", binary())
            result = inventory.inspect(path)
            self.assertTrue(result["has_arm64"])
            self.assertEqual(result["bundle_id"], "test.example")
            self.assertEqual(result["platform"], "appletvos")
            self.assertEqual(result["sdk"], "appletvos17.2")
            self.assertEqual(result["device_family"], [3])
            self.assertEqual(result["supported_platforms"], ["AppleTVOS"])
            self.assertEqual(len(result["sha256"]), 64)
            path.write_bytes(b"corrupt")
            self.assertIn("error", inventory.inspect(path))

    def test_duplicate_executable_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "test.ipa"
            with zipfile.ZipFile(path, "w") as archive:
                archive.writestr("Payload/Test.app/Info.plist", plistlib.dumps({"CFBundleExecutable": "Missing"}))
            self.assertIn("error", inventory.inspect(path))

    def test_blank_names_fall_back(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "Fallback.ipa"
            for bundle_name, expected in ((" Real Name ", "Real Name"), ("", "Fallback")):
                with zipfile.ZipFile(path, "w") as archive:
                    archive.writestr("Payload/Test.app/Info.plist", plistlib.dumps({"CFBundleExecutable": "Test", "CFBundleDisplayName": " \t", "CFBundleName": bundle_name}))
                    archive.writestr("Payload/Test.app/Test", binary())
                self.assertEqual(inventory.inspect(path)["name"], expected)


if __name__ == "__main__":
    unittest.main()
