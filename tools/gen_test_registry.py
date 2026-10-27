#!/usr/bin/env python3
"""Generate tools/test_registry.json (the ArenaOS test inventory).

The registry is the single machine-readable inventory consumed by
tools/arena_test.py. It is *generated* so suite metadata (docstring
purpose, imports, guest/host nature) cannot drift from the code, but
the registered-suite decomposition of tools/run_tests.sh and all
classification overrides below are explicit, reviewable data — edit
them here, rerun this script, then check with
``python3 tools/arena_test.py check-registry``.

How suites are classified:
* guest vs host: transitive import of tools/mtest.py (direct or via a
  sibling test module), with explicit overrides for suites that launch
  guests through subprocesses.
* mutates_source: lexical RED-control markers (NEEDLE/MUTANT + source
  write) plus overrides. Mutating suites restore byte-exact and assert
  it; the flag exists so the runner never runs two of them against one
  writable checkout (parallel workers get private worktrees).
* parallel: "host" (may share a checkout), "worktree" (needs an
  isolated checkout + namespaces), "serial" (never parallel).
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TOOLS = ROOT / "tools"

# Suites that execute a guest without (transitively) importing mtest in
# this file (they drive other suites/oracles as subprocesses).
GUEST_OVERRIDES = {
    "test_m85_ref_hook.py",   # runs test_m85_select fixture in a subprocess
    "test_m9_font_red.py",    # runs the QMP pixel oracle suite as subprocess
    "test_m11_repaint_red.py",  # host cargo RED/GREEN (desktop lib) — NOT guest; keep host below
}
GUEST_OVERRIDES.discard("test_m11_repaint_red.py")
# Suites that mutate production source (and restore it byte-exact):
MUTATE_OVERRIDES = {
    "test_m10_multi_red.py",       # multi-file replaced-source RED loop
    "test_m85_ref_hook.py",
    "test_m9_font_red.py",
    "test_m11_repaint_red.py",     # mutates desktop library source (host tests)
}
HOST_KIND_OVERRIDES = {
    "test_m11_repaint_red.py": "host-rust",
    "test_m10_ui.py": "host-rust",
    "test_m9_compositor_model.py": "host-python",
    "test_m9_gfxkit.py": "host-rust",
    "test_m9_gpu2d_red.py": "host-rust",
    "test_m9_gpu2d_wire.py": "host-rust",
}
# Advisory outer timeouts (the harness itself has per-boot deadlines).
TIMEOUT_OVERRIDES = {
    "test_m84_crash.py": 1800,      # 34 boots
    "test_m85_crash.py": 1800,
    "test_m85_crash_upgrade.py": 1800,
    "test_m82_policy_crash.py": 1800,
    "test_m9_client_death_red.py": 1200,
    "test_m11_afs2.py": 1200,
    "test_phase13_registry_guest.py": 1200,
}

PY = "python3"


def doc_title(src: str) -> str:
    m = re.match(r'(?:#!/usr/bin/env python3\n)?"""(.*?)"""', src, re.S)
    if not m:
        return ""
    text = " ".join(m.group(1).split())
    return text[:160]


def imports_of(src: str) -> set[str]:
    mods = set(re.findall(r"^(?:import|from)\s+([a-z0-9_]+)", src, re.M))
    mods |= set(re.findall(r"import[^\n]*\b(mtest|arena_env)\b", src))
    return mods


def main() -> int:
    sources = {p.stem: p.read_text(errors="replace")
               for p in sorted(TOOLS.glob("test_*.py"))}
    guest = {k: ("mtest" in imports_of(v)) for k, v in sources.items()}
    changed = True
    while changed:
        changed = False
        for k, v in sources.items():
            if not guest[k] and any(guest.get(m) for m in imports_of(v)
                                    if m.startswith("test_")):
                guest[k] = True
                changed = True
    for name in GUEST_OVERRIDES:
        guest[Path(name).stem] = True

    def mutates(name: str, src: str) -> bool:
        if name in MUTATE_OVERRIDES:
            return True
        return bool(re.search(
            r"MUTANT|NEEDLE|write_bytes\(original\.replace|"
            r"\.replace\(NEEDLE|SOURCE\.write_|"
            r"path\.write_bytes\([^\n]*\.replace", src))

    def neg_controls(name: str, title: str) -> str:
        if name.endswith("_red.py") or "red_control" in name:
            return ("intentional RED negative control: deliberately broken "
                    "production mechanism must fail in-guest (suite PASS means "
                    "RED was observed and restored GREEN passed)")
        if "negative" in name or "refusal" in name or "wrong_root" in name:
            return "capability/authority negative cases must be refused"
        return ""

    entries: list[dict] = []

    # ---- tools/run_tests.sh decomposition: 16 host blocks --------------
    # Commands are copied 1:1 from run_tests.sh (same order). If that
    # script changes, update here and in registered_count; the
    # check-registry gate enforces the count, not the text.
    def block(eid, title, cmd, kind="host-rust", cwd=".", sources_=(),
              quick=False, timeout=1200, paths=()):
        entries.append({
            "id": eid, "title": title, "kind": kind, "cmd": cmd,
            "cwd": cwd, "sources": list(sources_),
            "qualification": "registered", "quick": quick,
            "mutates_source": False, "parallel": "host",
            "timeout_s": timeout, "imports": [], "paths": list(paths),
            "negative_controls": "", "from": "tools/run_tests.sh host block",
        })

    block("host-heap", "Host unit tests: arena-heap (cargo test --lib)",
          ["cargo", "test", "-p", "arena-heap", "--lib", "--target",
           "x86_64-unknown-linux-gnu"], cwd="kernel", quick=True, timeout=600,
          paths=["kernel/libs/heap/**"])
    block("host-sync", "Host unit tests: arena-sync (cargo test --lib)",
          ["cargo", "test", "-p", "arena-sync", "--lib", "--target",
           "x86_64-unknown-linux-gnu"], cwd="kernel", quick=True, timeout=600,
          paths=["kernel/libs/sync/**"])
    block("host-dns-parser", "Host DNS codec tests (netstackd dns.rs)",
          ["bash", "-c", "mkdir -p build && rustc --test --edition 2024 "
           "userspace/netstackd/src/dns.rs -o build/dns-parser-tests && "
           "build/dns-parser-tests"], timeout=600,
          paths=["userspace/netstackd/src/dns.rs"])
    block("host-tcp-parser", "Host TCP wire/timeout policy tests (netstackd tcp.rs)",
          ["bash", "-c", "rustc --test --edition 2024 "
           "userspace/netstackd/src/tcp.rs -o build/tcp-parser-tests && "
           "build/tcp-parser-tests"], timeout=600,
          paths=["userspace/netstackd/src/tcp.rs"])
    block("host-net-api", "Host networking API contract tests (fake transport)",
          ["bash", "-c", "rustc --test --edition 2024 tools/net_api_test.rs "
           "-o build/net-api-tests && build/net-api-tests"], timeout=600)
    block("host-arena-lib", "Phase 8.3 userspace client libs: host tests + no_std target build",
          ["bash", "-c", "cargo test --manifest-path userspace/arena-lib/Cargo.toml "
           "--lib --target x86_64-unknown-linux-gnu && cargo build "
           "--manifest-path userspace/arena-lib/Cargo.toml --release "
           "--target x86_64-unknown-none"],
          paths=["userspace/arena-lib/**"])
    block("host-servicemgr-manifest", "Service manifest/inventory policy tests + no_std rlib",
          ["bash", "-c", "rustc --test --edition 2024 userspace/servicemgr/src/lib.rs "
           "-o build/service-manifest-tests && build/service-manifest-tests && "
           "rustc --crate-type lib --edition 2024 -D warnings "
           "userspace/servicemgr/src/lib.rs -o build/libarena-service-manifest.rlib"],
          timeout=600, paths=["userspace/servicemgr/src/lib.rs"])
    block("host-config-record", "Phase 8.1 config record codec + AFS1 integrity-limit probes",
          ["bash", "-c", "rustc --test --edition 2024 -D warnings userspace/config.rs "
           "-o build/config-record-tests && build/config-record-tests && "
           "rustc --crate-type lib --edition 2024 -D warnings --target "
           "x86_64-unknown-none --emit=metadata tools/config_no_std.rs "
           "-o build/config-no-std.rmeta && python3 tools/test_config_record.py && "
           "python3 tools/probe_config_commit_fallback.py"],
          sources_=["tools/test_config_record.py"], quick=True,
          paths=["userspace/config.rs", "tools/config_record.py"])
    block("host-permission-record", "Phase 8.2 immutable permission decision record",
          ["bash", "-c", "rustc --test --edition 2024 -D warnings userspace/permission.rs "
           "-o build/permission-record-tests && build/permission-record-tests && "
           "rustc --crate-type lib --edition 2024 -D warnings --target "
           "x86_64-unknown-none --emit=metadata tools/permission_no_std.rs "
           "-o build/permission-no-std.rmeta && python3 tools/test_permission_record.py"],
          sources_=["tools/test_permission_record.py"], quick=True,
          paths=["userspace/permission.rs", "tools/permission_record.py"])
    block("host-capspace-layout", "Phase 8.2 cap/IPC layout audit (host + bare-metal bounds)",
          ["python3", "tools/probe_capspace_layout.py"], kind="host-python",
          quick=True, timeout=600)
    block("host-phase85-records", "Phase 8.5 AINS/AACT records + no_std compile",
          ["bash", "-c", "python3 tools/test_phase85_records.py && rustc "
           "--crate-type lib --edition 2024 -D warnings --target "
           "x86_64-unknown-none --emit=metadata tools/installed_no_std.rs "
           "-o build/installed-no-std.rmeta"],
          sources_=["tools/test_phase85_records.py"], quick=True, timeout=600)
    # host-afs2 is deliberately NOT quick: the crash-prefix model takes
    # ~210 s on the reference sandbox (measured; see BASELINE.md).
    block("host-afs2", "AFS2 host model + Rust engine proofs (crash prefixes, RED)",
          ["bash", "-c", "python3 test_afs2.py && python3 test_afs2.py --red && "
           "python3 test_afs2_rust.py"], cwd="tools", kind="host-python",
          sources_=["tools/test_afs2.py", "tools/test_afs2_rust.py"],
          quick=False, paths=["tools/afs2.py", "userspace/afs2/**"])
    block("host-watch", "Phase 11 directory-watch host table + RED controls",
          ["python3", "test_watch_red.py"], cwd="tools", kind="host-python",
          sources_=["tools/test_watch_red.py"], quick=True, timeout=600)
    block("host-face13", "Phase 11.7 Arena Sans 13 generated-face sync check",
          ["python3", "tools/gen_face13.py", "--check"], kind="host-python",
          quick=True, timeout=600)
    block("host-phase12", "Phase 12 APB1/startup-ABI/native-runtime host+target suites",
          ["bash", "-c",
           "python3 tools/test_package_record.py && "
           "python3 tools/test_apb1_format.py && "
           "python3 tools/test_startup_abi.py && "
           "(cd userspace/arena-platform && cargo fmt -- --check && cargo test "
           "--manifest-path Cargo.toml --lib --target x86_64-unknown-linux-gnu "
           "--locked && cargo clippy --lib --target x86_64-unknown-linux-gnu "
           "--locked -- -D warnings && cargo build --manifest-path Cargo.toml "
           "--release --target x86_64-unknown-none --locked) && "
           "(cd userspace/arena-runtime && cargo fmt -- --check && cargo test "
           "--manifest-path Cargo.toml --lib --target x86_64-unknown-linux-gnu "
           "--locked && cargo clippy --lib --target x86_64-unknown-linux-gnu "
           "--locked -- -D warnings && cargo build --manifest-path Cargo.toml "
           "--bin arena-startup-proof --release --locked)"],
          sources_=["tools/test_package_record.py", "tools/test_apb1_format.py",
                    "tools/test_startup_abi.py"], timeout=1800,
          paths=["userspace/arena-platform/**", "userspace/arena-runtime/**",
                 "tools/apb1_format.py", "tools/package_record.py",
                 "tools/startup_abi.py"])
    block("host-phase13", "Phase 13 Desktop + service-manager host/target suites",
          ["bash", "-c",
           "cargo fmt --manifest-path userspace/desktop/Cargo.toml -- --check && "
           "cargo test --manifest-path userspace/desktop/Cargo.toml --lib "
           "--target x86_64-unknown-linux-gnu --locked && cargo clippy "
           "--manifest-path userspace/desktop/Cargo.toml --lib --target "
           "x86_64-unknown-linux-gnu --locked -- -D warnings && "
           "rustfmt --check --edition 2024 userspace/servicemgr/src/package.rs "
           "userspace/servicemgr/src/inventory.rs && cargo test --manifest-path "
           "userspace/servicemgr/Cargo.toml --lib --target "
           "x86_64-unknown-linux-gnu --locked && cargo check --manifest-path "
           "userspace/servicemgr/Cargo.toml --target x86_64-unknown-none --locked"],
          timeout=1800, paths=["userspace/desktop/**", "userspace/servicemgr/**"])

    # ---- registered guest entries also named in run_tests.sh -----------
    def guest_file_entry(fname: str, qualification: str = "registered",
                         quick: bool = False) -> dict:
        stem = Path(fname).stem
        src = sources.get(stem, "")
        is_guest = guest.get(stem, False)
        kind = HOST_KIND_OVERRIDES.get(fname) or (
            "guest-desktop" if "desktop=True" in src else
            "guest" if is_guest else "host-python")
        entry = {
            "id": stem.removeprefix("test_").replace("_", "-"),
            "title": doc_title(src),
            "kind": kind,
            "cmd": [PY, f"tools/{fname}"],
            "cwd": ".",
            "sources": [f"tools/{fname}"],
            "qualification": qualification,
            "quick": quick,
            "mutates_source": mutates(fname, src),
            "parallel": "worktree" if is_guest else "host",
            "timeout_s": TIMEOUT_OVERRIDES.get(fname, 1200 if is_guest else 900),
            "imports": sorted(m for m in imports_of(src) if m.startswith("test_")),
            "paths": [],
            "negative_controls": neg_controls(fname, doc_title(src)),
            "from": "tools/run_tests.sh (tools/test_m*.py glob or named)",
        }
        return entry

    entries.append(guest_file_entry("test_phase13_registry_guest.py"))
    entries[-1]["from"] = "tools/run_tests.sh (named Phase 13 guest block)"
    for f in ("test_phase14_pie_guest.py", "test_phase14_negative_guest.py",
              "test_phase14_no_entropy_guest.py",
              "test_phase14_stale_image_guest.py"):
        e = guest_file_entry(f)
        e["from"] = "tools/run_tests.sh (named Phase 14 loop)"
        entries.append(e)

    # ---- the 100 registered test_m*.py suites (alphabetical, as the
    # shell glob in run_tests.sh expands) -------------------------------
    for p in sorted(TOOLS.glob("test_m*.py")):
        e = guest_file_entry(p.name)
        if p.name == "test_m1.py":
            e["id"] = "smoke-m1"
            e["quick"] = True
            e["title"] = ("Milestone-1 guest markers (also the quick-mode "
                          "smoke boot; replaying every in-guest suite)")
        entries.append(e)

    # ---- standalone suites not in the registered 121 -------------------
    standalone = {
        "test_package_vendor.py": "host-python",
        "test_phase12_apb1_guest.py": "guest-desktop",
        "test_phase84_zip_unpack.py": "host-python",
        "test_phase85_design_capacity.py": "host-python",
        "test_phase85_elf_fit.py": "host-python",
        "test_phase85_ref_model.py": "host-python",
    }
    for fname, kind in standalone.items():
        e = guest_file_entry(fname, qualification="standalone")
        e["kind"] = kind
        e["from"] = ("not invoked by tools/run_tests.sh (invoked by other "
                     "suites or run standalone); registered here so the "
                     "inventory is complete")
        entries.append(e)

    # ---- Phase 16 host battery (outside the registered 121) -----------
    def p16host(eid, title, cmd, timeout=900):
        entries.append({
            "id": eid, "title": title, "kind": "host-c", "cmd": cmd,
            "cwd": ".", "sources": ["experiments/c-runtime/run.py"],
            "qualification": "phase16-host", "quick": False,
            "mutates_source": False, "parallel": "host",
            "timeout_s": timeout, "imports": [],
            "paths": ["experiments/c-runtime/src/**",
                      "experiments/c-runtime/include/**",
                      "experiments/c-runtime/app/**",
                      "experiments/c-runtime/host/**"],
            "negative_controls": "", "from": "Phase 16 canonical host battery",
        })

    p16host("p16-host-c-build-audit-gcc",
            "C runtime guest-ELF build + loader/ISA audit (GCC)",
            ["bash", "-c", "python3 experiments/c-runtime/run.py build --cc gcc && "
             "python3 experiments/c-runtime/run.py audit --cc gcc"])
    p16host("p16-host-c-build-audit-clang",
            "C runtime guest-ELF build + loader/ISA audit (Clang via zig cc)",
            ["bash", "-c", "python3 experiments/c-runtime/run.py build --cc clang && "
             "python3 experiments/c-runtime/run.py audit --cc clang"])
    p16host("p16-host-c-abi", "C/Rust ABI constant cross-check (92 constants)",
            ["python3", "experiments/c-runtime/run.py", "abi"], timeout=300)
    p16host("p16-host-c-host", "C runtime host suite under ASan+UBSan (LIBC-HOST + HOST-ONLY)",
            ["python3", "experiments/c-runtime/run.py", "host"], timeout=1200)
    p16host("p16-host-judge-selftests",
            "Guest judge negative controls (C2/FP/Notification/ISA-audit self-tests)",
            ["bash", "-c",
             "python3 experiments/c-runtime/guest_c2.py --self-test && "
             "python3 experiments/c-runtime/guest_fp.py --self-test && "
             "python3 experiments/c-runtime/guest_notification.py --self-test && "
             "python3 experiments/c-runtime/c2_isa_audit.py --self-test"],
            timeout=300)
    p16host("p16-host-fp-builds", "FP guest-app image builds + host mutant control (GCC/Clang)",
            ["bash", "-c", "python3 experiments/c-runtime/build_fp_app.py --cc gcc && "
             "python3 experiments/c-runtime/build_fp_app.py --cc clang"],
            timeout=600)

    # ---- Phase 16 targeted guest suites --------------------------------
    for suite in ("native", "c2", "fp", "stdin", "notification"):
        for cc in ("gcc", "clang"):
            entries.append({
                "id": f"p16-guest-{suite}-{cc}",
                "title": f"Phase 16 targeted guest suite '{suite}' ({cc.upper()})",
                "kind": "guest",
                "cmd": [PY, "-u", "experiments/c-runtime/guest_phase16_canonical.py",
                        "--suite", suite, "--", "--cc", cc],
                "cwd": ".",
                "sources": ["experiments/c-runtime/guest_phase16_canonical.py",
                            f"experiments/c-runtime/guest_{'native' if suite == 'native' else suite}.py"],
                "qualification": "phase16-targeted", "quick": False,
                "mutates_source": suite == "notification",
                "parallel": "worktree", "timeout_s": 900, "imports": [],
                "paths": ["experiments/c-runtime/**",
                          "userspace/desktop/**" if suite == "notification" else ""],
                "negative_controls":
                    "in-guest negative cases per suite (see FINAL-REPORT §6)"
                    if suite in ("fp", "stdin", "notification") else "",
                "from": "Phase 16 canonical targeted guests",
            })
    for e in entries:
        e["paths"] = [p for p in e.get("paths", []) if p]

    # ---- qualification-only entries ------------------------------------
    entries.append({
        "id": "canonical-final-build",
        "title": "Canonical final build (tools/build.sh --image) producing the exact artifacts the strict/stability gates then consume",
        "kind": "build", "cmd": ["bash", "tools/build.sh", "--image"],
        "cwd": ".", "sources": ["tools/build.sh"],
        "qualification": "strict-phase15", "quick": False,
        "mutates_source": False, "parallel": "serial", "timeout_s": 1800,
        "imports": [], "paths": [], "negative_controls": "",
        "from": "qualification orchestration (existing canonical build)",
    })
    entries.append({
        "id": "strict-m15-final",
        "title": "Strict Phase 15 guest check on the final build/ artifacts, both CPU profiles (genuine vector 19)",
        "kind": "guest", "cmd": [PY, "tools/test_m15_guest.py"],
        "cwd": ".", "sources": ["tools/test_m15_guest.py"],
        "qualification": "strict-phase15", "quick": False,
        "mutates_source": False, "parallel": "serial", "timeout_s": 600,
        "imports": [], "paths": [], "negative_controls":
            "unpatched-backend discrimination is established separately by the independent XM controls",
        "from": "Phase 15/16 strict receipts (same unchanged checker)",
    })
    entries.append({
        "id": "strict-m15-preserved",
        "title": "Strict Phase 15 guest check on the preserved accepted Phase 15 package artifacts",
        "kind": "guest",
        "cmd": ["bash", "-c",
                "ARENA_PHASE15_ESP=\"${ARENA_PHASE15_PKG:-$HOME/phase15-pkg}/artifacts/arena-esp.img\" "
                "ARENA_PHASE15_EFI=\"${ARENA_PHASE15_PKG:-$HOME/phase15-pkg}/artifacts/arena-boot.efi\" "
                "python3 tools/test_m15_guest.py"],
        "cwd": ".", "sources": ["tools/test_m15_guest.py"],
        "qualification": "strict-phase15", "quick": False,
        "mutates_source": False, "parallel": "serial", "timeout_s": 600,
        "imports": [], "paths": [], "negative_controls": "",
        "from": "Phase 15/16 strict receipts (preserved package pair)",
    })
    entries.append({
        "id": "stability-100",
        "title": "Final-artifact stability: 100 independent fresh boots of the exact build/ EFI/ESP, fully green",
        "kind": "stability", "cmd": ["bash", "tools/stability_loop.sh", "100"],
        "cwd": ".", "sources": ["tools/stability_loop.sh"],
        "qualification": "stability", "quick": False,
        "mutates_source": False, "parallel": "serial", "timeout_s": 3700,
        "imports": [], "paths": [], "negative_controls":
            "any single non-green boot invalidates the attempt (script semantics unchanged)",
        "from": "docs/TESTING.md milestone commit gate",
    })
    entries.append({
        "id": "fresh-archive-phase16",
        "title": "Fresh-extraction verification of the sealed Phase 16 demonstration archive (static + guest)",
        "kind": "package",
        "cmd": [PY, "tools/fresh_archive_verify.py", "--archive",
                "docs/phase16/artifacts/ArenaOS-Phase16-Demonstration-5aacbc8.tar.gz"],
        "cwd": ".", "sources": ["tools/fresh_archive_verify.py"],
        "qualification": "package", "quick": False,
        "mutates_source": False, "parallel": "serial", "timeout_s": 900,
        "imports": [], "paths": [], "negative_controls": "",
        "from": "Phase 16 fresh-archive receipts flow",
    })
    entries.append({
        "id": "backend-xm-controls",
        "title": "Independent #XM backend controls: patched QEMU must PASS, unpatched must expected-FAIL",
        "kind": "guest",
        "cmd": [PY, "tools/backend_controls.py"],
        "cwd": ".", "sources": ["tools/backend_controls.py"],
        "qualification": "strict-phase15", "quick": False,
        "mutates_source": False, "parallel": "serial", "timeout_s": 600,
        "imports": [], "paths": [],
        "negative_controls": "unpatched binary is the intentional negative control",
        "from": "Phase 15 backend qualification (verify_phase15_xm_controls.py)",
    })

    # ---- infrastructure self-tests -------------------------------------
    entries.append({
        "id": "infra-selftest",
        "title": "Testing-infrastructure controls (cache invalidation, evidence collision, fail-closed, semantic verdicts)",
        "kind": "host-python", "cmd": [PY, "tools/test_arena_test_infra.py"],
        "cwd": ".", "sources": ["tools/test_arena_test_infra.py"],
        "qualification": "infra", "quick": True,
        "mutates_source": False, "parallel": "host", "timeout_s": 900,
        "imports": [],
        "paths": ["tools/arena_test.py", "tools/build_cache.py",
                  "tools/mtest.py", "tools/test_registry.json",
                  "tools/test_map.json", "tools/gen_test_registry.py",
                  "tools/backend_controls.py", "tools/fresh_archive_verify.py"],
        "negative_controls": "the suite is itself a set of negative controls",
        "from": "testing workstream",
    })

    registered = sum(1 for e in entries if e["qualification"] == "registered")
    registry = {
        "schema": 1,
        "generated_by": "tools/gen_test_registry.py",
        "registered_count": registered,
        "registered_reference": "tools/run_tests.sh (official mode)",
        "entries": entries,
    }
    out = TOOLS / "test_registry.json"
    out.write_text(json.dumps(registry, indent=1) + "\n")
    print(f"wrote {out}: {len(entries)} entries, {registered} registered")
    if registered != 121:
        print("WARNING: registered count != 121 (run_tests.sh parity broken)",
              file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
