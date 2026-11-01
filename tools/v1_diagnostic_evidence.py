#!/usr/bin/env python3
"""Losslessly package diagnostic evidence, including failed/incomplete V1 runs.

This tool is intentionally separate from ``v1_evidence.py``. Qualification
packaging remains strict; this tool records what happened (including absent
frames and failed stages) without turning missing evidence into a pass.

Usage:
  python3 tools/v1_diagnostic_evidence.py seal \
    --evidence docs/v1/evidence/v2-2026-10-31-boundary-r6 \
    --packaging docs/v1/evidence/v2-2026-10-31-boundary-r6-packaging \
    --output docs/v1/evidence/v2-2026-10-31-boundary-r6-diagnostic.tar.gz
  python3 tools/v1_diagnostic_evidence.py verify <archive.tar.gz>

The diagnostic archive includes every file in the supplied run and packaging
roots (including raw PPM captures), immutable source snapshots from the
recorded source commit, and a report with stage/checkpoint/requirement verdicts
and exact missing-file paths. It never evaluates or changes the qualification
verdict. Scratch/build inputs that were not preserved by the run must be
identified in the run's ``omitted-inputs.json`` rather than silently presented
as if they were included.
"""

from __future__ import annotations

import argparse
import datetime as dt
import gzip
import hashlib
import io
import json
import os
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path, PurePosixPath
from typing import Any

REPO_ROOT = Path(__file__).resolve().parent.parent
SCHEMA = "arena.v1-diagnostic-evidence/v1"
REQUIRED_SOURCE_FILES = (
    "tools/v1_guest_proof.py",
    "tools/v1_refusals.py",
    "tools/v1_evidence.py",
    "tools/v1_gate.sh",
    "tools/v1_gate_stage.py",
    "tools/v1_gate_aggregate.py",
    "tools/test_phase13_registry_guest.py",
    "tools/mtest.py",
    "tools/qmp.py",
    "tools/arena_env.py",
    "experiments/c-runtime/app/c_gfx_app.c",
    "userspace/desktop/src/bin/desktop.rs",
)
REQUIRED_GUEST_FRAMES = (
    "shots/cp01-launch-ready-desktop.png",
    "shots/cp03-first-native-c-window.png",
    "shots/cp04-keyboard-response.png",
    "shots/cp05-mouse-response.png",
    "shots/cp06a-two-instances.png",
    "shots/cp06a-focused.png",
    "shots/cp06b-focused.png",
    "shots/cp07-desktop-restored.png",
    "shots/cp08-final-demonstration.png",
)


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as f:
        for block in iter(lambda: f.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def _read_json(path: Path) -> dict[str, Any] | None:
    try:
        value = json.loads(path.read_text())
    except (OSError, ValueError):
        return None
    return value if isinstance(value, dict) else None


def _git_show(commit: str, rel: str) -> bytes:
    result = subprocess.run(
        ["git", "show", f"{commit}:{rel}"], cwd=REPO_ROOT,
        capture_output=True, check=False, timeout=30,
    )
    if result.returncode:
        raise RuntimeError(
            f"cannot retrieve {rel} from source commit {commit}: "
            f"{result.stderr.decode(errors='replace').strip()}"
        )
    return result.stdout


def _receipt_roots(evidence: Path, packaging: Path) -> list[Path]:
    return [packaging / "gate" / "receipts", evidence / "gate" / "receipts"]


def _find_build_receipt(evidence: Path, packaging: Path) -> dict[str, Any]:
    for root in _receipt_roots(evidence, packaging):
        value = _read_json(root / "build.json")
        if value:
            return value
    return {}


def _stage_receipts(evidence: Path, packaging: Path) -> dict[str, dict[str, Any]]:
    result: dict[str, dict[str, Any]] = {}
    for root in _receipt_roots(evidence, packaging):
        if not root.is_dir():
            continue
        for path in sorted(root.glob("*.json")):
            value = _read_json(path)
            if value:
                result.setdefault(value.get("stage", path.stem), value)
    return result


def _parse_requirements(aggregate: str) -> list[dict[str, str]]:
    rows = []
    pattern = re.compile(r"^\s*(\d+(?:[a-g])?)\s+(PASS|FAIL|OPEN)\s+(.+?)\s*$")
    for line in aggregate.splitlines():
        match = pattern.match(line)
        if match:
            rows.append({"id": match.group(1), "status": match.group(2),
                         "coverage": match.group(3)})
    return rows


def _guest_summaries(evidence: Path) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    runs: dict[str, Any] = {}
    missing_files: list[dict[str, Any]] = []
    for compiler in ("gcc", "clang"):
        run_dir = evidence / f"guest-{compiler}"
        path = run_dir / "evidence.json"
        data = _read_json(path) or {}
        frames = data.get("frames") if isinstance(data.get("frames"), dict) else {}
        required = data.get("required_frames")
        if not isinstance(required, list) or not all(isinstance(x, str) for x in required):
            required = list(REQUIRED_GUEST_FRAMES)
        checkpoints = data.get("checkpoints")
        if not isinstance(checkpoints, list):
            checkpoints = []
        for frame in required:
            target = run_dir / frame
            if not target.is_file() or target.stat().st_size == 0:
                missing_files.append({
                    "compiler": compiler,
                    "path": str((Path(f"guest-{compiler}") / frame).as_posix()),
                    "present": False,
                    "checkpoint": next((item.get("case") for item in checkpoints
                                         if item.get("evidence_ref") == frame), None),
                })
        runs[compiler] = {
            "evidence_present": path.is_file(),
            "run_verdict": (data.get("verdict") or {}).get("pass")
                if isinstance(data.get("verdict"), dict) else None,
            "run": data.get("run", {}),
            "checkpoints": [
                {key: item.get(key) for key in ("case", "name", "result", "evidence_ref", "detail")}
                for item in checkpoints if isinstance(item, dict)
            ],
            "required_frames": required,
            "missing_required_frames": [x["path"] for x in missing_files
                                        if x["compiler"] == compiler],
            "available_probe_frames": sorted(
                str(p.relative_to(run_dir).as_posix())
                for p in (run_dir / "shots").glob("*")
                if p.is_file() and ("probe" in p.name or "ack" in p.name or "mouse" in p.name)
            ) if (run_dir / "shots").is_dir() else [],
            "source_artifact_identities": data.get("identities", {}),
            "frame_digest_entries": frames,
        }
    return runs, missing_files


def _integration_summaries(evidence: Path) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for compiler in ("gcc", "clang"):
        path = evidence / f"layout-integration-{compiler}" / "integration-evidence.json"
        data = _read_json(path) or {}
        fixtures = data.get("fixtures") if isinstance(data.get("fixtures"), dict) else {}
        result[compiler] = {
            "pass": data.get("pass"),
            "source_commit": data.get("source_commit"),
            "efi_sha256": (data.get("identities") or {}).get("boot_efi", {}).get("sha256"),
            "esp_sha256": (data.get("identities") or {}).get("esp", {}).get("sha256"),
            "fixtures": {},
        }
        for pages in ("301", "300"):
            fixture = fixtures.get(pages, {})
            rendered = (fixture.get("frames") or {}).get("rendered", {})
            result[compiler]["fixtures"][pages] = {
                "pass": fixture.get("pass"),
                "allocated_pages": fixture.get("allocated_pages"),
                "client_reported_pages": fixture.get("client_reported_pages"),
                "retired_pages": fixture.get("retired_pages"),
                "client_exit_status": fixture.get("client_exit_status"),
                "identify_surface_object_id": fixture.get("identify_surface_object_id"),
                "window_created": fixture.get("window_created"),
                "no_map_write_window_damage_before_refusal": fixture.get(
                    "no_map_write_window_damage_before_refusal"),
                "rendered_visual_assertion": fixture.get("rendered_visual_assertion"),
                "rendered_visual_bands": fixture.get("rendered_visual_bands"),
                "rendered_frame_assertion_description": rendered.get("assertion"),
                "resource_drift": fixture.get("resource_drift"),
                "artifact_identities": {
                    key: fixture.get(key) for key in (
                        "apb1_sha256", "elf_sha256", "source_commit",
                        "boot_efi_sha256", "esp_sha256") if key in fixture
                },
                "captures": fixture.get("frames", {}),
            }
    return result


def _command_transcripts(evidence: Path, packaging: Path) -> dict[str, str]:
    result: dict[str, str] = {}
    for label, root in (("r6", evidence), ("package-attempt", packaging)):
        if not root.is_dir():
            continue
        for path in sorted(root.rglob("commands.txt")):
            if path.is_symlink() or not path.is_file():
                continue
            key = f"{label}/{path.relative_to(root).as_posix()}"
            result[key] = path.read_text(errors="replace")
    return result


def _runtime_identity_files(evidence: Path) -> dict[str, str]:
    paths = (
        "env/host.txt",
        "env/isolated-runtime.txt",
        "env/qemu-identities.txt",
        "env/qemu-version.txt",
        "env/qemu-package.txt",
        "env/rust-cargo-version.txt",
        "env/rust-component-checksums.txt",
        "env/compilers/compiler-binary-identities.txt",
        "env/compilers/tool-identities.txt",
    )
    result = {}
    for rel in paths:
        path = evidence / rel
        if path.is_file() and not path.is_symlink():
            result[rel] = path.read_text(errors="replace")
    return result


def build_report(evidence: Path, packaging: Path) -> dict[str, Any]:
    evidence = evidence.resolve()
    packaging = packaging.resolve()
    build = _find_build_receipt(evidence, packaging)
    receipts = _stage_receipts(evidence, packaging)
    aggregate_path = packaging / "gate" / "logs" / "final-aggregate.log"
    aggregate = aggregate_path.read_text(errors="replace") if aggregate_path.is_file() else ""
    gate_match = re.search(r"V1 GATE \(full\):\s*(PASS|FAIL)", aggregate)
    observed_gate = gate_match.group(1) if gate_match else "NOT_RECORDED"
    guest_runs, missing_files = _guest_summaries(evidence)
    phase13_timeout = evidence / "regression" / "phase13" / "timeout-diagnostics.txt"
    timeout_text = phase13_timeout.read_text(errors="replace") if phase13_timeout.is_file() else ""
    omitted = _read_json(evidence / "omitted-inputs.json") or {"omitted": []}
    commands = evidence / "gate" / "commands.txt"
    command_text = commands.read_text(errors="replace") if commands.is_file() else ""
    stage_results = {
        stage: {
            "exit": receipt.get("exit"),
            "qualified": receipt.get("qualified"),
            "started_utc": receipt.get("started_utc"),
            "finished_utc": receipt.get("finished_utc"),
            "argv": receipt.get("argv", []),
            "notes": receipt.get("notes", []),
        }
        for stage, receipt in sorted(receipts.items())
    }
    return {
        "schema": SCHEMA,
        "package_classification": "diagnostic-only",
        "qualification_claim": "NONE",
        "candidate_qualified": False,
        "observed_full_gate_result": observed_gate,
        "explicit_warning": (
            "This package preserves a failed/incomplete run. It is not a "
            "qualification package and must not be interpreted as V1 acceptance."
        ),
        "source_commit": build.get("source_commit"),
        "branch": "arena/af044828-arenaos",
        "artifact_identities": {
            key: build.get(key) for key in (
                "code_fingerprint", "efi_sha256", "esp_sha256") if build.get(key)
        },
        "phase14_identities_from_build_log": {
            "protected_pie_sha256": "d7b0a8cf9c735c3898a867d824563f06b0d949df80fa1a9c96f9180a395fea2e",
            "signed_apb1_sha256": "04add54f09042353fc42511978c82f8b8efb87f7d5f0657f9116f456276b3dfa",
            "identity_source": "included gate/logs/build.log; source commit 64efe9b858f8dce10b186ce7f80b8f47d041292d",
        },
        "runtime_identity_evidence": [
            "env/host.txt", "env/isolated-runtime.txt", "env/qemu-identities.txt",
            "env/qemu-version.txt", "env/qemu-package.txt",
            "env/rust-cargo-version.txt", "env/rust-component-checksums.txt",
            "env/compilers/compiler-binary-identities.txt",
            "env/compilers/tool-identities.txt",
        ],
        "runtime_identity": _runtime_identity_files(evidence),
        "full_gate_command_transcript": command_text,
        "command_transcripts": _command_transcripts(evidence, packaging),
        "stage_results": stage_results,
        "requirement_matrix": _parse_requirements(aggregate),
        "guest_runs": guest_runs,
        "missing_checkpoint_files": missing_files,
        "real_grant_integrations": _integration_summaries(evidence),
        "phase13_timeout": {
            "suite": "tools/test_phase13_registry_guest.py",
            "configured_timeout_seconds": 600,
            "timeout_report": timeout_text,
            "qmp_result_classification": (
                "Timed out with TimeoutError; the old timeout helper did not "
                "record which QMP handshake/command was blocked. No CPU/RIP "
                "sample was obtained; the exact QMP failure stage is unknown."
            ),
            "raw_files": [
                "regression/phase13/serial-timeout.log",
                "regression/phase13/serial-post-kill.log",
                "regression/phase13/virtconsole-timeout.log",
                "regression/phase13/qemu-stderr.log",
                "regression/phase13/timeout-diagnostics.txt",
                "regression/phase13/setup/seed/",
                "regression/phase13/setup/afs2/",
                "regression/phase13/captures/",
            ],
        },
        "packaging_failure": {
            "qualification_packager": "tools/v1_evidence.py (strict, unchanged in this diagnostic package)",
            "package_log": "gate/logs/package.log",
            "verify_log": "gate/logs/verify.log",
            "qualification_archive_present": (evidence.parent / f"{evidence.name}.tar.gz").is_file(),
            "qualification_checksum_present": (evidence.parent / f"{evidence.name}.tar.gz.sha256").is_file(),
        },
        "historical_capture_reconciliation": {
            "report": "historical-capture-reconciliation/analysis.md",
            "current_and_committed_bytes_preserved": True,
            "binary_diff_preserved": True,
            "historical_paths_restored_from_HEAD_after_preservation": True,
            "newer_harness_reused_historical_path": False,
            "actor_of_text_transcoding": "unknown; not recorded in surviving logs",
        },
        "omitted_inputs": omitted,
        "source_snapshots": {},
        "archive_contents": {
            "raw_ppm_captures_included": True,
            "png_captures_included": True,
            "raw_logs_included": True,
            "qualification_failure_logs_included": True,
            "missing_captures_are_recorded_not_created": True,
        },
    }


def _copy_tree_no_links(src: Path, dst: Path) -> None:
    if not src.is_dir():
        raise ValueError(f"evidence input is not a directory: {src}")
    for path in sorted(src.rglob("*")):
        if path.is_symlink():
            raise ValueError(f"refusing symlink in diagnostic evidence: {path}")
        rel = path.relative_to(src)
        target = dst / rel
        if path.is_dir():
            target.mkdir(parents=True, exist_ok=True)
        elif path.is_file():
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(path, target)


def _classify(rel: str) -> str:
    p = PurePosixPath(rel)
    if p.parts and p.parts[0] == "source":
        return "source-snapshot"
    if p.suffix.lower() in (".ppm", ".png", ".jpg", ".jpeg", ".webp"):
        return "raw-or-lossless-capture"
    if p.suffix.lower() in (".log", ".txt"):
        return "raw-log-or-run-transcript"
    if p.suffix.lower() in (".efi", ".img", ".fd", ".elf", ".apb1"):
        return "build-or-runtime-artifact"
    if p.suffix.lower() == ".json":
        return "receipt-or-structured-evidence"
    return "diagnostic-record-or-source-metadata"


def _write_snapshot_sources(stage_root: Path, source_commit: str) -> dict[str, str]:
    hashes = {}
    source_root = stage_root / "source" / source_commit
    for rel in REQUIRED_SOURCE_FILES:
        data = _git_show(source_commit, rel)
        target = source_root / rel
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)
        hashes[rel] = sha256_bytes(data)
    # The packager itself is a working-tree helper introduced after the r6
    # candidate commit. Include it explicitly with a clearly separate identity.
    tool_source = Path(__file__).read_bytes()
    target = stage_root / "source" / "diagnostic-packager-working-tree" / Path(__file__).name
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(tool_source)
    hashes[f"diagnostic-packager-working-tree/{Path(__file__).name}"] = sha256_bytes(tool_source)
    return hashes


def _walk_files(root: Path) -> list[Path]:
    result = []
    for path in sorted(root.rglob("*")):
        if path.is_symlink():
            raise ValueError(f"refusing symlink in staged diagnostic archive: {path}")
        if path.is_file():
            result.append(path)
    return result


def _make_readme(report: dict[str, Any], manifest: dict[str, Any], root_name: str) -> str:
    missing = report.get("missing_checkpoint_files", [])
    missing_lines = "\n".join(
        f"- `{row['path']}` (compiler `{row['compiler']}`, checkpoint `{row.get('checkpoint')}`)"
        for row in missing
    ) or "- None recorded."
    reqs = report.get("requirement_matrix", [])
    req_lines = "\n".join(
        f"| {row['id']} | {row['status']} | {row['coverage']} |" for row in reqs
    ) or "| unavailable | NOT RECORDED | See included aggregate log |"
    stages = report.get("stage_results", {})
    stage_lines = "\n".join(
        f"| {stage} | exit={entry.get('exit')} | {entry.get('finished_utc')} |"
        for stage, entry in stages.items()
    ) or "| unavailable | no receipts | |"
    return f"""# {root_name} — diagnostic evidence only

> **Qualification verdict: NOT QUALIFIED.** The observed full gate result is
> **{report.get('observed_full_gate_result')}**. This package is diagnostic
> evidence only; it makes no acceptance, qualification, merge, or release
> claim. Missing frames are listed as missing and are never synthesized.

## Run identity

- Source commit: `{report.get('source_commit')}`
- Branch at run: `{report.get('branch')}`
- Code fingerprint: `{report.get('artifact_identities', {}).get('code_fingerprint')}`
- EFI SHA-256: `{report.get('artifact_identities', {}).get('efi_sha256')}`
- ESP SHA-256: `{report.get('artifact_identities', {}).get('esp_sha256')}`
- Qualification status: NOT QUALIFIED; r6 classification remains failed.
- Raw PPM captures: included verbatim (not filtered like the qualification packager).
- Archive entries: {len(manifest['files'])}; original file bytes: {manifest['total_bytes']}.

Runtime versions and hashes are summarized in `DIAGNOSTIC-REPORT.json`
(`runtime_identity`) and retained verbatim under `run/r6/env/`. Exact commands
are preserved in `DIAGNOSTIC-REPORT.json` (`command_transcripts`,
`full_gate_command_transcript`, and stage `argv` fields), with their original
`commands.txt` files under the run roots. Source snapshots are under
`source/{report.get('source_commit')}/`.

## Stage verdicts

| Stage | Result | Finished UTC |
|---|---:|---|
{stage_lines}

The authoritative package and verification failures are retained at
`run/package-attempt/gate/logs/package.log` and
`run/package-attempt/gate/logs/verify.log`; the final aggregate is
`run/package-attempt/gate/logs/final-aggregate.log`.

## Requirement matrix

| Requirement | Verdict | Coverage |
|---|---|---|
{req_lines}

Requirement 4b remains OPEN: the recorded `foreign-identity` test mutates
identity bytes and is not a genuine foreign/stale capability launch. No M
waiver is present. Predicate-only proof (5e) and real-grant proof (5f) remain
distinct.

## Guest checkpoint verdicts and missing captures

The complete compiler-specific checkpoint records are in
`DIAGNOSTIC-REPORT.json` and the original `run/r6/guest-{{gcc,clang}}/evidence.json`.
The GCC pointer checkpoint remains a FAIL: `{report['guest_runs'].get('gcc', {}).get('checkpoints', [{}])[4].get('detail', 'not recorded') if len(report['guest_runs'].get('gcc', {}).get('checkpoints', [])) > 4 else 'checkpoint verdict unavailable'}`.

Missing required GCC captures:
{missing_lines}

The package also preserves `__ack-clear__.ppm`, the available final
`__ack-probe__.ppm`, and the pre-input mouse frame. Their presence is not
represented as a passing response frame.

## Phase 13 timeout

The installed-application guest exceeded its existing 600-second timeout.
The raw serial and virtconsole logs, QEMU stderr, setup boots, 30 progress PPM
captures, and original QMP timeout report are retained under
`run/r6/regression/phase13/`. The r6 helper recorded only
`TimeoutError('timed out')`; it did not record which QMP handshake/command was
blocked, and obtained no CPU/RIP sample. Therefore this package does not claim
to distinguish a QMP-client defect from a QEMU/event-loop stall.

## Layout real-grant evidence

The GCC and Clang integration JSON and QEMU captures are retained under
`run/r6/layout-integration-{{gcc,clang}}/`. The summary in
`DIAGNOSTIC-REPORT.json` records separate 301/300 results, page counts,
rendered band assertions, geometry-related fields, capture identities, and
resource drift. Screenshots supplement—not replace—the numeric and serial
proof.

## Historical PPM reconciliation

The four historical refusal-matrix files' current bytes, committed originals,
blob/SHA identities, and binary diff are preserved under
`run/r6/historical-capture-reconciliation/`. The working-tree bytes were an
exact UTF-8 replacement-character transformation of committed PPM bytes, not
new screenshots. The original historical paths were restored only after both
states were preserved. The exact process that caused the text transformation
is unknown; no surviving evidence attributes it to a newer harness.

## Omitted reproducible inputs

Read `run/r6/omitted-inputs.json`. It distinguishes reproducible runtimes/build
outputs from raw evidence. No raw log or capture was omitted to reduce size.
The current `build/scratch.img` hash is explicitly **not** the Phase 13 run
input: the original timed-out disk was overwritten by later gate work and its
run-time hash was not recorded.

## Independent archive verification

The archive manifest covers every file except the manifest itself. Verify from
a fresh extraction with:

```sh
python3 tools/v1_diagnostic_evidence.py verify {root_name}.tar.gz
```

This verifies diagnostic archive integrity only. It does not run the strict
qualification checks and cannot change any stage or requirement verdict.
"""


def _tar_add_file(tar: tarfile.TarFile, root: Path, path: Path) -> None:
    rel = f"{root.name}/{path.relative_to(root).as_posix()}"
    info = tarfile.TarInfo(rel)
    info.size = path.stat().st_size
    info.mode = 0o644
    info.mtime = 0
    info.uid = 0
    info.gid = 0
    info.uname = ""
    info.gname = ""
    with path.open("rb") as f:
        tar.addfile(info, f)


def create_archive(evidence: Path, packaging: Path, archive: Path,
                   repo_root: Path = REPO_ROOT) -> dict[str, Any]:
    evidence = evidence.resolve()
    packaging = packaging.resolve()
    archive = archive.resolve()
    if not evidence.is_dir() or not packaging.is_dir():
        raise ValueError("both --evidence and --packaging must name existing directories")
    if archive == evidence or archive == packaging or evidence in archive.parents or packaging in archive.parents:
        raise ValueError("archive must be outside both source evidence trees")
    build = _find_build_receipt(evidence, packaging)
    source_commit = build.get("source_commit")
    if not isinstance(source_commit, str) or not source_commit:
        raise ValueError("no source_commit in the diagnostic run's build receipt")
    report = build_report(evidence, packaging)
    if report["qualification_claim"] != "NONE" or report["candidate_qualified"] is not False:
        raise ValueError("diagnostic report must never claim qualification")

    archive.parent.mkdir(parents=True, exist_ok=True)
    root_name = archive.name[:-7] if archive.name.endswith(".tar.gz") else archive.stem
    with tempfile.TemporaryDirectory(prefix="v1-diagnostic-stage-") as tmp:
        staging = Path(tmp) / root_name
        staging.mkdir()
        run_root = staging / "run"
        _copy_tree_no_links(evidence, run_root / "r6")
        _copy_tree_no_links(packaging, run_root / "package-attempt")
        source_hashes = _write_snapshot_sources(staging, source_commit)
        report["source_snapshots"] = source_hashes
        report_path = staging / "DIAGNOSTIC-REPORT.json"
        report_path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
        readme_path = staging / "README.md"
        # First pass counts archive files; README does not depend on its final
        # digest and the manifest includes the README and report contents.
        files = _walk_files(staging)
        inventory = {
            path.relative_to(staging).as_posix(): {
                "size": path.stat().st_size,
                "sha256": sha256_file(path),
                "classification": _classify(path.relative_to(staging).as_posix()),
            }
            for path in files
        }
        total_bytes = sum(row["size"] for row in inventory.values())
        manifest = {
            "schema": SCHEMA,
            "package_classification": "diagnostic-only",
            "qualification_claim": "NONE",
            "candidate_qualified": False,
            "observed_full_gate_result": report["observed_full_gate_result"],
            "source_commit": source_commit,
            "branch": report["branch"],
            "created_from_evidence": str(evidence),
            "created_from_packaging_logs": str(packaging),
            "files": inventory,
            "file_count": len(inventory),
            "total_bytes": total_bytes,
            "raw_ppm_count": sum(1 for rel in inventory if rel.lower().endswith(".ppm")),
            "omitted_inputs_report": "run/r6/omitted-inputs.json",
            "missing_required_frames": report["missing_checkpoint_files"],
            "warning": "Diagnostic integrity only; no qualification PASS is asserted or inferred.",
        }
        readme_path.write_text(_make_readme(report, manifest, root_name))
        # Include README in the manifest after it is finalized. Manifest itself
        # is intentionally not self-listed; verify rejects any other unlisted
        # payload.
        inventory["README.md"] = {
            "size": readme_path.stat().st_size,
            "sha256": sha256_file(readme_path),
            "classification": "diagnostic-report",
        }
        manifest["files"] = inventory
        manifest["file_count"] = len(inventory)
        manifest["total_bytes"] = sum(row["size"] for row in inventory.values())
        manifest_path = staging / "MANIFEST.json"
        manifest_path.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
        files = _walk_files(staging)
        with archive.open("wb") as raw:
            with gzip.GzipFile(fileobj=raw, mode="wb", filename="", mtime=0,
                               compresslevel=6) as gz:
                with tarfile.open(fileobj=gz, mode="w", format=tarfile.PAX_FORMAT) as tar:
                    dir_info = tarfile.TarInfo(root_name)
                    dir_info.type = tarfile.DIRTYPE
                    dir_info.mode = 0o755
                    dir_info.mtime = 0
                    dir_info.uid = dir_info.gid = 0
                    dir_info.uname = dir_info.gname = ""
                    tar.addfile(dir_info)
                    for path in sorted(files):
                        _tar_add_file(tar, staging, path)
        archive_hash = sha256_file(archive)
        sidecar = archive.with_suffix(archive.suffix + ".sha256")
        sidecar.write_text(f"{archive_hash}  {archive.name}\n")
        return {
            "archive": str(archive),
            "archive_bytes": archive.stat().st_size,
            "archive_sha256": archive_hash,
            "files": manifest["file_count"],
            "total_bytes": manifest["total_bytes"],
            "raw_ppm_count": manifest["raw_ppm_count"],
            "missing_required_frames": len(manifest["missing_required_frames"]),
            "sidecar": str(sidecar),
            "manifest": manifest,
        }


def verify_archive(archive: Path) -> tuple[int, list[str]]:
    archive = archive.resolve()
    if not archive.is_file():
        return 2, [f"archive not found: {archive}"]
    problems: list[str] = []
    try:
        with tempfile.TemporaryDirectory(prefix="v1-diagnostic-verify-") as tmp:
            with tarfile.open(archive, "r:gz") as tar:
                for member in tar.getmembers():
                    member_path = PurePosixPath(member.name)
                    if member_path.is_absolute() or ".." in member_path.parts:
                        return 1, [f"unsafe tar member: {member.name}"]
                    if not (member.isdir() or member.isfile()):
                        return 1, [f"unsupported link/special member: {member.name}"]
                try:
                    tar.extractall(tmp, filter="data")
                except TypeError:
                    # Older Python versions lack extraction filters. Every member
                    # has already been checked for traversal, absolute paths,
                    # symlinks and special files above.
                    tar.extractall(tmp)
            roots = [p for p in Path(tmp).iterdir() if p.is_dir()]
            if len(roots) != 1:
                return 1, [f"expected one archive root, got {len(roots)}"]
            root = roots[0]
            manifest = _read_json(root / "MANIFEST.json")
            if not manifest or manifest.get("schema") != SCHEMA:
                return 1, ["missing or unsupported diagnostic MANIFEST.json"]
            if manifest.get("package_classification") != "diagnostic-only":
                problems.append("archive is not explicitly classified diagnostic-only")
            if manifest.get("qualification_claim") != "NONE" or manifest.get("candidate_qualified") is not False:
                problems.append("diagnostic archive contains a qualification claim")
            expected = manifest.get("files")
            if not isinstance(expected, dict):
                return 1, ["MANIFEST.json files map is absent"]
            for rel, want in expected.items():
                path = root / rel
                if not path.is_file():
                    problems.append(f"{rel}: missing")
                    continue
                if path.stat().st_size != want.get("size"):
                    problems.append(f"{rel}: byte size mismatch")
                elif sha256_file(path) != want.get("sha256"):
                    problems.append(f"{rel}: SHA-256 mismatch")
            actual = {
                path.relative_to(root).as_posix()
                for path in root.rglob("*") if path.is_file() and path.name != "MANIFEST.json"
            }
            listed = set(expected)
            for rel in sorted(actual - listed):
                problems.append(f"{rel}: unlisted file in diagnostic archive")
            for rel in sorted(listed - actual):
                if not any(problem.startswith(f"{rel}:") for problem in problems):
                    problems.append(f"{rel}: listed file absent from archive")
    except (OSError, tarfile.TarError, ValueError, EOFError) as exc:
        return 1, [f"cannot verify diagnostic archive: {type(exc).__name__}: {exc}"]
    return (0 if not problems else 1), problems


def seal(evidence: Path, packaging: Path, archive: Path) -> int:
    try:
        result = create_archive(evidence, packaging, archive)
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as exc:
        print(f"[diagnostic] package failed: {type(exc).__name__}: {exc}", file=sys.stderr)
        return 1
    print(f"[diagnostic] diagnostic-only archive: {result['archive']}")
    print(f"[diagnostic] qualification result remains: {result['manifest']['observed_full_gate_result']}")
    print(f"[diagnostic] {result['files']} files / {result['total_bytes']} original bytes; "
          f"{result['raw_ppm_count']} PPM captures; {result['missing_required_frames']} required captures missing")
    print(f"[diagnostic] archive sha256={result['archive_sha256']} bytes={result['archive_bytes']}")
    rc, problems = verify_archive(Path(result["archive"]))
    print(f"[diagnostic] fresh local extraction verification: {'PASS' if rc == 0 else 'FAIL'}")
    for problem in problems:
        print(f"[diagnostic] PROBLEM: {problem}")
    if rc:
        return rc
    manifest = result["manifest"]
    receipt = [
        "ARENA V1 DIAGNOSTIC ARCHIVE RECEIPT",
        "Classification: DIAGNOSTIC ONLY — NOT A QUALIFICATION PACKAGE",
        f"Observed full gate result: {manifest['observed_full_gate_result']}",
        f"Candidate qualified: {manifest['candidate_qualified']}",
        f"Source commit: {manifest['source_commit']}",
        f"Branch: {manifest['branch']}",
        f"Archive: {Path(result['archive']).name}",
        f"Archive size: {result['archive_bytes']} bytes",
        f"Archive SHA-256: {result['archive_sha256']}",
        f"Manifest files: {result['files']}",
        f"Raw PPM captures included: {result['raw_ppm_count']}",
        f"Missing required captures recorded: {result['missing_required_frames']}",
        "Fresh local extraction verification: PASS (every listed digest checked; no unlisted files)",
        f"Verification command: python3 tools/v1_diagnostic_evidence.py verify {Path(result['archive']).name}",
        f"Sealed UTC: {dt.datetime.now(dt.timezone.utc).isoformat()}",
    ]
    archive_path = Path(result["archive"])
    (archive_path.parent / (archive_path.name + ".receipt.txt")).write_text("\n".join(receipt) + "\n")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    package_parser = sub.add_parser("package", help="build a diagnostic-only archive; failed qualification is allowed")
    package_parser.add_argument("--evidence", type=Path, required=True)
    package_parser.add_argument("--packaging", type=Path, required=True)
    package_parser.add_argument("--output", type=Path, required=True)
    seal_parser = sub.add_parser("seal", help="package and verify from a fresh temporary extraction")
    seal_parser.add_argument("--evidence", type=Path, required=True)
    seal_parser.add_argument("--packaging", type=Path, required=True)
    seal_parser.add_argument("--output", type=Path, required=True)
    verify_parser = sub.add_parser("verify", help="verify a diagnostic-only archive from fresh extraction")
    verify_parser.add_argument("archive", type=Path)
    args = parser.parse_args(argv)
    if args.command == "verify":
        rc, problems = verify_archive(args.archive)
        print(f"[diagnostic] verify: {'PASS' if rc == 0 else 'FAIL'}")
        for problem in problems:
            print(f"[diagnostic] PROBLEM: {problem}")
        return rc
    if args.command == "seal":
        return seal(args.evidence, args.packaging, args.output)
    try:
        result = create_archive(args.evidence, args.packaging, args.output)
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as exc:
        print(f"[diagnostic] package failed: {type(exc).__name__}: {exc}", file=sys.stderr)
        return 1
    print(f"[diagnostic] packaged failed-run evidence: {result['archive']} "
          f"({result['archive_bytes']} bytes, sha256 {result['archive_sha256']}); "
          f"qualification not asserted")
    return 0


if __name__ == "__main__":
    sys.exit(main())
