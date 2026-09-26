#!/usr/bin/env python3
"""ArenaOS dev-environment resolution (docs/DEV-ENV.md).

Locates the Rust toolchain, QEMU, and UEFI firmware (OVMF/EDK2) across the
two supported environments:

1. The offline sandbox: toolchain under /opt/rust/prefix, QEMU under /opt/qemu
   (static musl build, launched through /opt/musl/lib/libc.so as its dynamic
   loader because the host lacks ld-musl and its glibc is too old for the
   glibc build), firmware under /opt/qemu/share/qemu.
2. A normal workstation: rustup toolchain on PATH, qemu-system-x86_64 on
   PATH, distro OVMF (/usr/share/OVMF, /usr/share/edk2, ...).

Environment overrides (highest priority):
  ARENA_RUST_BIN    directory containing rustc/cargo
  ARENA_QEMU        full command prefix for qemu-system-x86_64 (space-split)
  ARENA_OVMF_CODE   path to OVMF/edk2 code flash (x86_64)
  ARENA_OVMF_VARS   path to OVMF/edk2 vars flash template
"""

import os
import shutil
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

SANDBOX_RUST = Path("/opt/rust/prefix/bin")
SANDBOX_QEMU_BIN = Path("/opt/qemu/bin/qemu-system-x86_64")
SANDBOX_QEMU_LIB = Path("/opt/qemu/lib")
SANDBOX_QEMU_SHARE = Path("/opt/qemu/share/qemu")
SANDBOX_MUSL_LOADER = Path("/opt/musl/lib/libc.so")

OVMF_SEARCH_DIRS = [
    SANDBOX_QEMU_SHARE,
    Path("/usr/share/OVMF"),
    Path("/usr/share/edk2/ovmf-x64"),
    Path("/usr/share/OVMF/x64"),
    Path("/usr/share/qemu"),
]
OVMF_CODE_NAMES = ["edk2-x86_64-code.fd", "OVMF_CODE.fd", "OVMF_CODE-pure-efi.fd", "ovmf_code_x64.bin"]
OVMF_VARS_NAMES = ["edk2-i386-vars.fd", "OVMF_VARS.fd", "OVMF_VARS-pure-efi.fd", "ovmf_vars_x64.bin"]


def rust_bin() -> Path:
    env = os.environ.get("ARENA_RUST_BIN")
    if env:
        return Path(env)
    if (SANDBOX_RUST / "cargo").exists():
        return SANDBOX_RUST
    if shutil.which("cargo"):
        return Path(shutil.which("cargo")).parent
    sys.exit("error: no Rust toolchain found (see tools/dev-env/bootstrap.sh)")


def rust_env() -> dict:
    """Environment with the resolved toolchain on PATH."""
    env = dict(os.environ)
    env["PATH"] = f"{rust_bin()}:{env['PATH']}"
    return env


def qemu_cmd() -> list[str]:
    """Full command prefix that launches qemu-system-x86_64."""
    env = os.environ.get("ARENA_QEMU")
    if env:
        return env.split()
    if SANDBOX_QEMU_BIN.exists() and SANDBOX_MUSL_LOADER.exists():
        # Static musl QEMU: launch through musl's libc.so, which doubles as
        # the dynamic loader; vendored libs come from /opt/qemu/lib.
        loader_env = f"LD_LIBRARY_PATH={SANDBOX_QEMU_LIB}"
        return ["env", loader_env, str(SANDBOX_MUSL_LOADER), str(SANDBOX_QEMU_BIN)]
    if shutil.which("qemu-system-x86_64"):
        return [shutil.which("qemu-system-x86_64")]
    sys.exit("error: no qemu-system-x86_64 found (see tools/dev-env/bootstrap.sh)")


def qemu_data_args() -> list[str]:
    """Extra args QEMU needs to find its ROMs (sandbox layout only)."""
    if SANDBOX_QEMU_SHARE.exists() and str(SANDBOX_QEMU_BIN) in " ".join(qemu_cmd()):
        return ["-L", str(SANDBOX_QEMU_SHARE)]
    return []


def _find_firmware(names: list[str]) -> Path | None:
    env_code = os.environ.get("ARENA_OVMF_CODE")
    env_vars = os.environ.get("ARENA_OVMF_VARS")
    envval = env_code if names is OVMF_CODE_NAMES else env_vars
    if envval:
        return Path(envval)
    for d in OVMF_SEARCH_DIRS:
        for n in names:
            p = d / n
            if p.exists():
                return p
    return None


def ovmf_code() -> Path:
    p = _find_firmware(OVMF_CODE_NAMES)
    if p is None:
        sys.exit("error: no OVMF/EDK2 code flash found (set ARENA_OVMF_CODE)")
    return p


def ovmf_vars_template() -> Path:
    p = _find_firmware(OVMF_VARS_NAMES)
    if p is None:
        sys.exit("error: no OVMF/EDK2 vars flash found (set ARENA_OVMF_VARS)")
    return p


def build_dir() -> Path:
    d = REPO_ROOT / "build"
    d.mkdir(exist_ok=True)
    return d


# ---- the Milestone-5 scratch disk (ADR-0021) --------------------------------
#
# The harness fixture every boot attaches as virtio-blk-pci: the kernel's
# PCI scan must find it (m5 pci_scan) and, from M5.2 on, the userspace
# storage driver reads/writes it. Fresh zero-filled image per run: until
# step 5.4 makes persistence an explicit two-boot test, no run may inherit
# another run's disk contents. The ESP stays the BOOT medium — this disk
# is never bootable.

SCRATCH_MIB = 8


def make_scratch_disk() -> Path:
    """Create (or re-create) the fresh zero-filled scratch disk image."""
    p = build_dir() / "scratch.img"
    with open(p, "wb") as f:
        f.truncate(SCRATCH_MIB * 1024 * 1024)
    return p


def scratch_disk_args() -> list[str]:
    """QEMU args attaching the fresh scratch disk as virtio-blk-pci."""
    p = make_scratch_disk()
    return [
        "-drive", f"file={p},format=raw,if=none,id=scr0",
        "-device", "virtio-blk-pci,drive=scr0",
    ]


if __name__ == "__main__":
    print("repo root :", REPO_ROOT)
    print("rust bin  :", rust_bin())
    print("qemu cmd  :", " ".join(qemu_cmd()))
    print("qemu data :", qemu_data_args())
    print("ovmf code :", ovmf_code())
    print("ovmf vars :", ovmf_vars_template())
