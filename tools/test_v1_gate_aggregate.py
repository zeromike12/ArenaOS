#!/usr/bin/env python3
"""Focused controls for the requirement-aware V1 gate aggregate."""

import hashlib
import json
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tools"))
import v1_gate_aggregate as aggregate  # noqa: E402


class AggregateRequirementTests(unittest.TestCase):
    def _evidence(self, root: Path, stage: str) -> dict:
        root.mkdir(parents=True, exist_ok=True)
        cases = {}
        classes = {}
        for name, contract in aggregate.LAYOUT_CASES.items():
            got = contract.get("got", contract.get("minimum_got"))
            if name == "layout-pages-minimum":
                got = 471  # actual guest grant is larger than the minimum
            cases[name] = {
                "count": 1,
                "want": contract["want"],
                "got": got,
                "verdict": "PASS",
            }
            classes[name] = {"kind": contract["kind"], "description": "fixture"}
        apb1 = root / "c-gfx-refusals.apb1"
        apb1.write_bytes(f"synthetic APB1 for {stage}".encode())
        evidence = {
            "guest_compiler_under_test": stage.rsplit("-", 1)[-1],
            "source_commit": "test-source-commit",
            "apb1_sha256": hashlib.sha256(apb1.read_bytes()).hexdigest(),
            "pass": True,
            "rc": 0,
            "record": {
                "verdict_ok": True,
                "unclassified": [],
                "cases": cases,
                "classes": classes,
            },
        }
        path = root / "refusals-evidence.json"
        path.write_text(json.dumps(evidence))
        return {
            "evidence_dir": str(root.relative_to(ROOT)),
            "source_commit": "test-source-commit",
        }

    def test_full_mode_is_derived_from_all_requirement_stages(self):
        required = {
            stage
            for requirement in aggregate.REQUIREMENTS.values()
            for stage in requirement["stages"]
        }
        self.assertTrue(required.issubset(set(aggregate.STAGES["full"])))
        self.assertIn("aggregate-controls", aggregate.STAGES["full"])
        self.assertIn("regression", aggregate.STAGES["full"])
        self.assertIn("stability", aggregate.STAGES["full"])

    def test_5e_accepts_both_compiler_cells_with_production_policy_cases(self):
        with tempfile.TemporaryDirectory(prefix=".v1-aggregate-", dir=ROOT) as tmp:
            root = Path(tmp)
            for stage in ("refusals-gcc", "refusals-clang"):
                receipt = self._evidence(root / stage, stage)
                self.assertEqual([], aggregate.layout_evidence_problems(receipt, stage))

    def test_5e_rejects_absent_300_page_refusal_even_if_stage_passed(self):
        with tempfile.TemporaryDirectory(prefix=".v1-aggregate-", dir=ROOT) as tmp:
            root = Path(tmp) / "refusals-gcc"
            receipt = self._evidence(root, "refusals-gcc")
            path = root / "refusals-evidence.json"
            data = json.loads(path.read_text())
            del data["record"]["cases"]["layout-300-refused"]
            path.write_text(json.dumps(data))
            errors = aggregate.layout_evidence_problems(receipt, "refusals-gcc")
            self.assertTrue(any("missing case layout-300-refused" in e for e in errors))

    def test_5e_rejects_wrong_300_page_result(self):
        with tempfile.TemporaryDirectory(prefix=".v1-aggregate-", dir=ROOT) as tmp:
            root = Path(tmp) / "refusals-gcc"
            receipt = self._evidence(root, "refusals-gcc")
            path = root / "refusals-evidence.json"
            data = json.loads(path.read_text())
            data["record"]["cases"]["layout-300-refused"]["got"] = 0
            path.write_text(json.dumps(data))
            errors = aggregate.layout_evidence_problems(receipt, "refusals-gcc")
            self.assertTrue(any("layout-300-refused got=0" in e for e in errors))

    def test_5e_rejects_kernel_misclassification_of_synthetic_policy_input(self):
        with tempfile.TemporaryDirectory(prefix=".v1-aggregate-", dir=ROOT) as tmp:
            root = Path(tmp) / "refusals-gcc"
            receipt = self._evidence(root, "refusals-gcc")
            path = root / "refusals-evidence.json"
            data = json.loads(path.read_text())
            data["record"]["classes"]["layout-300-refused"]["kind"] = "KERNEL"
            path.write_text(json.dumps(data))
            errors = aggregate.layout_evidence_problems(receipt, "refusals-gcc")
            self.assertTrue(any("expected 'CLIENT-POLICY'" in e for e in errors))

    def test_5e_rejects_evidence_from_the_wrong_compiler_cell(self):
        with tempfile.TemporaryDirectory(prefix=".v1-aggregate-", dir=ROOT) as tmp:
            root = Path(tmp) / "refusals-gcc"
            receipt = self._evidence(root, "refusals-gcc")
            path = root / "refusals-evidence.json"
            data = json.loads(path.read_text())
            data["guest_compiler_under_test"] = "clang"
            path.write_text(json.dumps(data))
            errors = aggregate.layout_evidence_problems(receipt, "refusals-gcc")
            self.assertTrue(any("expected 'gcc'" in e for e in errors))

    def test_5e_rejects_evidence_from_a_different_source_commit(self):
        with tempfile.TemporaryDirectory(prefix=".v1-aggregate-", dir=ROOT) as tmp:
            root = Path(tmp) / "refusals-gcc"
            receipt = self._evidence(root, "refusals-gcc")
            path = root / "refusals-evidence.json"
            data = json.loads(path.read_text())
            data["source_commit"] = "historical-source-commit"
            path.write_text(json.dumps(data))
            errors = aggregate.layout_evidence_problems(receipt, "refusals-gcc")
            self.assertTrue(any("does not match stage receipt" in e for e in errors))

    def test_5e_rejects_a_substituted_apb1_payload(self):
        with tempfile.TemporaryDirectory(prefix=".v1-aggregate-", dir=ROOT) as tmp:
            root = Path(tmp) / "refusals-gcc"
            receipt = self._evidence(root, "refusals-gcc")
            (root / "c-gfx-refusals.apb1").write_bytes(b"substituted payload")
            errors = aggregate.layout_evidence_problems(receipt, "refusals-gcc")
            self.assertTrue(any("does not match c-gfx-refusals.apb1" in e
                                for e in errors))

    def test_5e_requires_a_repo_local_evidence_directory(self):
        self.assertTrue(aggregate.layout_evidence_problems({}, "refusals-gcc"))
        self.assertTrue(aggregate.layout_evidence_problems(
            {"evidence_dir": "../../outside"}, "refusals-gcc"))

    def test_full_requirement_evaluation_fails_without_5e_case_evidence(self):
        receipts = {stage: {"exit": 0, "qualified": True}
                    for stage in aggregate.STAGES["full"]}
        with tempfile.TemporaryDirectory(prefix=".v1-aggregate-", dir=ROOT) as tmp:
            root = Path(tmp) / "refusals-gcc"
            receipt = self._evidence(root, "refusals-gcc")
            path = root / "refusals-evidence.json"
            data = json.loads(path.read_text())
            del data["record"]["cases"]["layout-300-refused"]
            path.write_text(json.dumps(data))
            receipts["refusals-gcc"].update(receipt)
            receipts["refusals-clang"].update(
                self._evidence(Path(tmp) / "refusals-clang", "refusals-clang"))
            statuses, errors = aggregate.evaluate_requirements(
                "full", receipts, set(), set())
        layout = next(item for item in statuses if item["id"] == "5e")
        self.assertEqual("FAIL", layout["status"])
        self.assertTrue(any("missing case layout-300-refused" in e for e in errors))


if __name__ == "__main__":
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(AggregateRequirementTests)
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    print(f"V1 AGGREGATE CONTROLS: {result.testsRun - len(result.failures) - len(result.errors)}"
          f"/{result.testsRun} PASS")
    sys.exit(0 if result.wasSuccessful() else 1)
