import unittest

from summarize_arm64_regression import summarize


class SummaryTests(unittest.TestCase):
    def test_exact_root_fixture_names(self):
        names = ["Arm64DylibTest.ipa", "Arm64InitializerTest.ipa", "Arm64SparseTest.ipa", "Arm64RealGame.ipa"]
        inventory = {"ipas": [{"sha256": name, "path": "C:/project/" + name} for name in names]}
        rows = summarize(inventory, {"results": []})["rows"]
        self.assertEqual([row["category"] for row in rows], ["fixture", "fixture", "fixture", "unknown platform"])

    def test_multiple_retests_keep_evidence_last_wins(self):
        inventory = {"ipas": [{"sha256": "same", "name": "App"}]}
        baseline = {"results": [{"sha256": "same", "boundary": "baseline", "log": "base.log"}]}
        retests = [{"results": [{"sha256": "same", "boundary": "first", "log": "first.log"}]},
                   {"results": [{"sha256": "same", "boundary": "latest", "log": "latest.log"}]}]
        row = summarize(inventory, baseline, retests)["rows"][0]
        self.assertEqual(row["boundary"], "latest")
        self.assertEqual(row["baseline"][0]["log"], "base.log")
        self.assertEqual([r["log"] for r in row["retest"]], ["first.log", "latest.log"])


if __name__ == "__main__":
    unittest.main()
