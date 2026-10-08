import unittest
from pathlib import Path
import tempfile

from regress_arm64 import fingerprint, file_digest, validate_build_proof, hashes


class FingerprintTests(unittest.TestCase):
    def test_relocated_same_failure_groups_together(self):
        left = "UIKit _objc_msgSend PC 0x100abc /Payload/One.app/main after 18 supervisor traps"
        right = "UIKit _objc_msgSend PC 0xABC900 /Payload/Two.app/main after 42 supervisor traps"
        self.assertEqual(fingerprint(left), fingerprint(right))

    def test_tlv_workload_size_is_not_a_new_failure(self):
        left = "TLV unsupported: 4 actual TLV images / 8 descriptors / 0 TLS initializer callbacks"
        right = "TLV unsupported: 12 actual TLV images / 97 descriptors / 3 TLS initializer callbacks"
        self.assertEqual(fingerprint(left), fingerprint(right))

    def test_distinct_api_and_provider_remain_distinct(self):
        base = "CoreFoundation _CFNumberGetValue at 0x1000"
        self.assertNotEqual(fingerprint(base), fingerprint("CoreFoundation _CFDictionaryGetValue at 0x2000"))
        self.assertNotEqual(fingerprint(base), fingerprint("Foundation _CFNumberGetValue at 0x2000"))

    def test_failure_reason_remains_distinct(self):
        self.assertNotEqual(fingerprint("_objc_release invalid identity 0x100"), fingerprint("_objc_release dealloc quarantine 0x200"))


class BuildProofTests(unittest.TestCase):
    def test_exact_content_and_tampering(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'src').mkdir()
            source = root / 'src/a64.rs'
            source.write_text('original source')
            binary, cargo = root / 'test-binary', root / 'build.json'
            binary.write_bytes(b'original executable')
            cargo.write_text('{"reason":"build-finished","success":true}')
            baseline = hashes(root)
            proof = {'schema': 1, 'build_succeeded': True, 'source_hashes_before': baseline,
                     'source_hashes_after': baseline.copy(), 'test_binary_sha256': file_digest(binary),
                     'build_json_sha256': file_digest(cargo)}
            validate_build_proof(proof, baseline, binary, cargo)
            source.write_text('new source with stale binary')
            with self.assertRaisesRegex(ValueError, 'current native snapshot'):
                validate_build_proof(proof, hashes(root), binary, cargo)
            source.write_text('original source')
            binary.write_bytes(b'changed executable')
            with self.assertRaisesRegex(ValueError, 'binary hash'):
                validate_build_proof(proof, hashes(root), binary, cargo)
            binary.write_bytes(b'original executable')
            cargo.write_text('different Cargo result')
            with self.assertRaisesRegex(ValueError, 'JSON hash'):
                validate_build_proof(proof, hashes(root), binary, cargo)

    def test_build_time_source_mutation_is_not_a_proof(self):
        proof = {'schema': 1, 'build_succeeded': True, 'source_hashes_before': {'src/a64.rs': 'before'},
                 'source_hashes_after': {'src/a64.rs': 'after'}}
        with self.assertRaisesRegex(ValueError, 'during build'):
            validate_build_proof(proof, {'src/a64.rs': 'after'}, Path('unused'), Path('unused'))


if __name__ == "__main__":
    unittest.main()
