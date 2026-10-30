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
        cases["foreign-identity"] = {
            "count": 1, "want": 0, "got": 0, "verdict": "PASS",
        }
        classes["foreign-identity"] = {
            "kind": "IDENTITY",
            "description": "production identity validator over an object id one above the real one",
        }
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

    def test_4b_keeps_synthetic_foreign_identity_open_without_m_waiver(self):
        with tempfile.TemporaryDirectory(prefix=".v1-aggregate-", dir=ROOT) as tmp:
            root = Path(tmp)
            receipts = {stage: {"exit": 0, "qualified": True}
                        for stage in aggregate.STAGES["full"]}
            for stage in ("refusals-gcc", "refusals-clang"):
                receipt = self._evidence(root / stage, stage)
                errors, waived = aggregate.foreign_capability_evidence_problems(
                    receipt, stage)
                self.assertEqual([], errors)
                self.assertFalse(waived)
                receipts[stage].update(receipt)
            statuses, _errors = aggregate.evaluate_requirements(
                "full", receipts, set(), set())
            foreign = next(item for item in statuses if item["id"] == "4b")
            self.assertEqual("OPEN", foreign["status"])

    def test_4b_does_not_accept_a_waiver_not_recorded_by_m(self):
        with tempfile.TemporaryDirectory(prefix=".v1-aggregate-", dir=ROOT) as tmp:
            root = Path(tmp)
            receipt = self._evidence(root / "refusals-gcc", "refusals-gcc")
            receipt["efi_sha256"] = "candidate-efi"
            (root / "M-WAIVER.json").write_text(json.dumps({
                "kind": "m_recorded_v1_coverage_waiver",
                "requirement": "4b",
                "decision": "waive",
                "decision_by": "someone-else",
                "scope": "no real stale/foreign capability is presented to the kernel",
                "source_commit": "test-source-commit",
                "efi_sha256": "candidate-efi",
                "recorded_utc": "2026-10-30T00:00:00Z",
                "rationale": "test fixture only",
            }))
            errors, waived = aggregate.foreign_capability_evidence_problems(
                receipt, "refusals-gcc")
            self.assertFalse(waived)
            self.assertTrue(any("decision_by" in error for error in errors))

    def test_5f_is_a_required_real_grant_stage_for_full_mode(self):
        receipts = {stage: {"exit": 0, "qualified": True}
                    for stage in aggregate.STAGES["full"]}
        for stage in ("layout-integration-gcc", "layout-integration-clang"):
            receipts.pop(stage)
        statuses, errors = aggregate.evaluate_requirements(
            "full", receipts,
            {"layout-integration-gcc", "layout-integration-clang"}, set())
        real_grant = next(item for item in statuses if item["id"] == "5f")
        self.assertEqual("FAIL", real_grant["status"])
        self.assertIn("missing layout-integration-gcc, layout-integration-clang",
                      real_grant["detail"])
        self.assertTrue(errors)

    def test_5f_rejects_predicate_only_evidence(self):
        with tempfile.TemporaryDirectory(prefix=".v1-aggregate-", dir=ROOT) as tmp:
            root = Path(tmp) / "layout-integration-gcc"
            root.mkdir()
            (root / "integration-evidence.json").write_text(json.dumps({
                "kind": "predicate_only_policy_proof",
                "guest_compiler_under_test": "gcc",
                "pass": True,
                "rc": 0,
            }))
            receipt = {"evidence_dir": str(root.relative_to(ROOT)),
                       "source_commit": "test-source-commit"}
            errors = aggregate.layout_integration_evidence_problems(
                receipt, "layout-integration-gcc")
            self.assertTrue(any("not real SharedRegion integration" in error
                                for error in errors))

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
