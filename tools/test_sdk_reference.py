from pathlib import Path
import tempfile
import unittest

from sdk_reference import SDKReference


class ReferenceTests(unittest.TestCase):
    def test_relative_chain_and_directory_link(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "real").mkdir()
            (root / "real/header.h").write_text("header")
            (root / "alias").write_text("real")
            (root / "link.h").write_text("alias/header.h")
            reference = SDKReference(root, {"real/header.h": "100644", "alias": "120000", "link.h": "120000"})
            self.assertEqual(reference.resolve("link.h"), root / "real/header.h")

    def test_escape_absolute_and_cycle_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "a").write_text("b")
            (root / "b").write_text("a")
            reference = SDKReference(root, {"a": "120000", "b": "120000"})
            with self.assertRaisesRegex(ValueError, "cycle"):
                reference.resolve("a")
            for target in ("../outside", "/outside", "C:/outside"):
                (root / "a").write_text(target)
                with self.assertRaises(ValueError):
                    reference.resolve("a")

    def test_untracked_plain_target_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "secret").write_text("not SDK metadata")
            reference = SDKReference(root, {})
            with self.assertRaises(ValueError):
                reference.resolve("secret")


if __name__ == "__main__":
    unittest.main()
