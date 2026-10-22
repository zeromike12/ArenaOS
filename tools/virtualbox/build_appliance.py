#!/usr/bin/env python3
"""Build a self-contained VirtualBox .vbox package from the interactive EFI.

The archive contains the native VM configuration, fixed VDI disks, a concise
README, and a hash manifest. It does not contain VirtualBox firmware:
VirtualBox supplies the EFI firmware on the host. Building the package does
not test guest execution.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import struct
import subprocess
import sys
import tempfile
import uuid
import zipfile
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
BUILD = ROOT / "build"
EFI = BUILD / "arena-virtualbox-interactive.efi"
ESP = BUILD / "arena-virtualbox-interactive-esp.img"
IDENTITY = BUILD / "arena-virtualbox-interactive.identity"
ARTIFACT = ROOT / "docs/virtualbox/artifacts/ArenaOS-VirtualBox-Interactive.zip"
SECTOR = 512
ESP_START_LBA = 2048
ESP_SECTORS = 16 * 1024 * 1024 // SECTOR
BOOT_DISK_SECTORS = ESP_START_LBA + ESP_SECTORS + 2048
DATA_DISK_MIB = 72
AFS1_MIB = 8
AFS2_BASE_SECTOR = 16384
VBOX_VERSION = "7.2.22"
EFI_PARTITION_TYPE = uuid.UUID("c12a7328-f81f-11d2-ba4b-00a0c93ec93b")
VDI_FILE_INFO = b"<<< Oracle VirtualBox Disk Image >>>\n"
VDI_SIGNATURE = 0xBEDA107F
VDI_VERSION = 0x00010001
VDI_HEADER_SIZE = 400
VDI_BLOCK_SIZE = 1024 * 1024
VDI_DATA_ALIGNMENT = 1024 * 1024
VDI_IMAGE_TYPE_FIXED = 2


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def guid(value: uuid.UUID) -> str:
    return "{" + str(value) + "}"


def uuid_for(label: str, identity: str) -> uuid.UUID:
    return uuid.uuid5(uuid.NAMESPACE_URL, f"https://arenaos.invalid/{label}/{identity}")


def check_esp() -> None:
    try:
        from pyfatfs.PyFatFS import PyFatFS
    except ImportError as exc:
        raise SystemExit("pyfatfs is required by the project ESP build tooling") from exc
    fs = PyFatFS(str(ESP), read_only=True)
    try:
        embedded = fs.getbytes("/EFI/BOOT/BOOTX64.EFI")
    finally:
        fs.close()
    if embedded != EFI.read_bytes():
        raise SystemExit("the EFI in the ESP does not match the interactive EFI")


def partition_entry(part_guid: uuid.UUID, start: int, end: int, name: str) -> bytes:
    raw = bytearray(128)
    raw[0:16] = EFI_PARTITION_TYPE.bytes_le
    raw[16:32] = part_guid.bytes_le
    struct.pack_into("<QQQ", raw, 32, start, end, 0)
    encoded_name = name.encode("utf-16le")[:72]
    raw[56 : 56 + len(encoded_name)] = encoded_name
    return bytes(raw)


def gpt_header(
    current: int,
    backup: int,
    table_lba: int,
    first_usable: int,
    last_usable: int,
    disk_guid: uuid.UUID,
    table_crc: int,
) -> bytes:
    header = struct.pack(
        "<8sIIIIQQQQ16sQIII",
        b"EFI PART",
        0x00010000,
        92,
        0,
        0,
        current,
        backup,
        first_usable,
        last_usable,
        disk_guid.bytes_le,
        table_lba,
        128,
        128,
        table_crc,
    )
    checksum = zlib.crc32(header) & 0xFFFF_FFFF
    header = bytearray(header)
    struct.pack_into("<I", header, 16, checksum)
    return bytes(header).ljust(SECTOR, b"\0")


def make_boot_disk(esp_bytes: bytes, identity: str, path: Path) -> None:
    total = BOOT_DISK_SECTORS
    last_lba = total - 1
    first_usable = 34
    last_usable = total - 34
    part_end = ESP_START_LBA + ESP_SECTORS - 1
    disk_id = uuid_for("boot-disk", identity)
    part_id = uuid_for("efi-partition", identity)

    table = bytearray(128 * 128)
    table[:128] = partition_entry(part_id, ESP_START_LBA, part_end, "ArenaOS EFI")
    table_crc = zlib.crc32(table) & 0xFFFF_FFFF
    primary_header = gpt_header(1, last_lba, 2, first_usable, last_usable, disk_id, table_crc)
    backup_table_lba = total - 33
    backup_header = gpt_header(
        last_lba,
        1,
        backup_table_lba,
        first_usable,
        last_usable,
        disk_id,
        table_crc,
    )

    image = bytearray(total * SECTOR)
    image[510:512] = b"\x55\xAA"
    image[446 + 4] = 0xEE
    image[446 + 1 : 446 + 4] = b"\x00\x02\x00"
    image[446 + 5 : 446 + 8] = b"\xFF\xFF\xFF"
    struct.pack_into("<II", image, 446 + 8, 1, total - 1)
    image[SECTOR : 2 * SECTOR] = primary_header
    image[2 * SECTOR : 2 * SECTOR + len(table)] = table
    image[backup_table_lba * SECTOR : (backup_table_lba + 32) * SECTOR] = table
    image[last_lba * SECTOR : (last_lba + 1) * SECTOR] = backup_header
    offset = ESP_START_LBA * SECTOR
    image[offset : offset + len(esp_bytes)] = esp_bytes
    path.write_bytes(image)


def make_data_disk(path: Path) -> None:
    sys.path.insert(0, str(ROOT / "tools"))
    import afs1

    eight_mib = AFS1_MIB * 1024 * 1024
    with path.open("wb") as stream:
        stream.truncate(eight_mib)
    afs1.mkfs(path, eight_mib // afs1.SECTOR)
    with path.open("r+b") as stream:
        stream.truncate(DATA_DISK_MIB * 1024 * 1024)


def write_vdi(
    path: Path,
    raw_path: Path,
    image_id: uuid.UUID,
    modification_id: uuid.UUID,
) -> None:
    """Write a fixed VDI 1.1 image around a sector-aligned raw disk.

    The layout follows VirtualBox 7.2.22's VDICore.h and VDI.cpp structures:
    a 72-byte preheader, 400-byte VDIHEADER1PLUS, 1 MiB-aligned block map and
    data area, and native RTUUID bytes for the image UUID.
    """
    disk_size = raw_path.stat().st_size
    if disk_size == 0 or disk_size % SECTOR:
        raise SystemExit(f"raw disk size is not a nonzero multiple of {SECTOR}: {raw_path}")
    if disk_size % VDI_BLOCK_SIZE:
        raise SystemExit(f"raw disk size is not a multiple of the VDI block size: {raw_path}")
    blocks = disk_size // VDI_BLOCK_SIZE
    off_blocks = (72 + VDI_HEADER_SIZE + VDI_DATA_ALIGNMENT - 1) // VDI_DATA_ALIGNMENT * VDI_DATA_ALIGNMENT
    off_data = (off_blocks + blocks * 4 + VDI_DATA_ALIGNMENT - 1) // VDI_DATA_ALIGNMENT * VDI_DATA_ALIGNMENT

    header = bytearray(VDI_HEADER_SIZE)
    struct.pack_into("<III", header, 0, VDI_HEADER_SIZE, VDI_IMAGE_TYPE_FIXED, 0)
    struct.pack_into("<II", header, 268, off_blocks, off_data)
    struct.pack_into("<IIII", header, 276, 0, 0, 0, SECTOR)
    struct.pack_into("<I", header, 292, 0)
    struct.pack_into("<QIIII", header, 296, disk_size, VDI_BLOCK_SIZE, 0, blocks, blocks)
    header[320:336] = image_id.bytes_le
    header[336:352] = modification_id.bytes_le
    struct.pack_into("<IIII", header, 384, 0, 0, 0, SECTOR)

    preheader = VDI_FILE_INFO.ljust(64, b"\0") + struct.pack("<II", VDI_SIGNATURE, VDI_VERSION)
    with path.open("wb") as output:
        output.write(preheader)
        output.write(header)
        output.write(b"\0" * (off_blocks - output.tell()))
        output.write(struct.pack(f"<{blocks}I", *range(blocks)))
        output.write(b"\0" * (off_data - output.tell()))
        with raw_path.open("rb") as raw:
            for chunk in iter(lambda: raw.read(1024 * 1024), b""):
                output.write(chunk)


def vbox_xml(
    machine_id: uuid.UUID,
    machine_name: str,
    data_id: uuid.UUID,
    boot_id: uuid.UUID,
    network_mac: str,
) -> str:
    return f'''<?xml version="1.0"?>
<!-- ArenaOS interactive VirtualBox profile; generated from the recorded build identity. -->
<VirtualBox xmlns="http://www.virtualbox.org/" version="1.20-linux">
  <Machine uuid="{guid(machine_id)}" name="{machine_name}" OSType="Other_64" snapshotFolder="Snapshots" lastStateChange="2026-10-22T00:00:00Z">
    <MediaRegistry>
      <HardDisks>
        <HardDisk uuid="{guid(data_id)}" location="ArenaOS-Data.vdi" format="VDI" type="Normal"/>
        <HardDisk uuid="{guid(boot_id)}" location="ArenaOS-Boot.vdi" format="VDI" type="Normal"/>
      </HardDisks>
    </MediaRegistry>
    <Platform architecture="x86">
      <RTC localOrUTC="UTC"/>
      <Chipset type="ICH9"/>
      <CPU count="1" hotplug="false"/>
      <x86>
        <CPU>
          <HardwareVirtExLargePages enabled="false"/>
          <HardwareVirtExNestedPaging enabled="true"/>
          <HardwareVirtExVPID enabled="true"/>
          <HardwareVirtExUX enabled="true"/>
          <PAE enabled="false"/>
          <LongMode enabled="true"/>
          <X2APIC enabled="false"/>
        </CPU>
      </x86>
    </Platform>
    <Hardware version="2">
      <Memory RAMSize="1024" PageFusion="false"/>
      <HID Pointing="PS2Mouse" Keyboard="PS2Keyboard"/>
      <Boot>
        <Order position="1" device="HardDisk"/>
        <Order position="2" device="None"/>
        <Order position="3" device="None"/>
        <Order position="4" device="None"/>
      </Boot>
      <Display controller="VBoxVGA" VRAMSize="16" monitorCount="1" accelerate3D="false" accelerate2DVideo="false"/>
      <Firmware type="EFI" secureBootEnabled="false">
        <ACPI enabled="true"/>
        <IOAPIC enabled="true"/>
        <Logo fadeIn="false" fadeOut="false" displayTime="0"/>
        <BootMenu mode="MessageAndMenu"/>
        <TimeOffset value="0"/>
      </Firmware>
      <UART>
        <Port slot="0" enabled="true" IOAddress="0x3f8" IRQ="4" hostMode="RawFile" path="ArenaOS-serial.log"/>
      </UART>
      <Network>
        <Adapter slot="0" enabled="true" MACAddress="{network_mac}" cable="true" type="virtio" promiscuousModePolicy="Deny">
          <NAT/>
        </Adapter>
        <Adapter slot="1" enabled="false" type="82540EM"/>
        <Adapter slot="2" enabled="false" type="82540EM"/>
        <Adapter slot="3" enabled="false" type="82540EM"/>
      </Network>
      <AudioAdapter enabled="false" driver="Default" enabledOut="false" enabledIn="false"/>
      <Clipboard mode="Disabled"/>
      <DragAndDrop mode="Disabled"/>
      <StorageControllers>
        <StorageController name="SATA" type="AHCI" PortCount="2" useHostIOCache="false" Bootable="true">
          <AttachedDevice type="HardDisk" hotpluggable="false" port="0" device="0">
            <Image uuid="{guid(data_id)}"/>
          </AttachedDevice>
          <AttachedDevice type="HardDisk" hotpluggable="false" port="1" device="0">
            <Image uuid="{guid(boot_id)}"/>
          </AttachedDevice>
        </StorageController>
      </StorageControllers>
    </Hardware>
  </Machine>
</VirtualBox>
'''


def read_identity() -> dict[str, str]:
    if not IDENTITY.is_file():
        raise SystemExit("build the interactive EFI first: tools/build.sh --virtualbox-interactive --image")
    result: dict[str, str] = {}
    for line in IDENTITY.read_text(encoding="utf-8").splitlines():
        if "=" in line:
            key, value = line.split("=", 1)
            result[key] = value
    if result.get("profile") != "virtualbox-interactive":
        raise SystemExit("the recorded build identity is not the interactive profile")
    return result


def build(output: Path, allow_dirty: bool) -> None:
    for path in (EFI, ESP):
        if not path.is_file():
            raise SystemExit(f"missing required build output: {path}")
    check_esp()
    identity = read_identity()
    status_lines = subprocess.run(
        ["git", "-C", str(ROOT), "status", "--porcelain", "--untracked-files=all"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.splitlines()
    output_rel = output.relative_to(ROOT).as_posix() if output.is_relative_to(ROOT) else None
    # The generated ZIP may already exist or be staged for an artifact-only
    # refresh. It is derived output; source changes remain subject to the
    # clean-tree guard.
    source_status = [line for line in status_lines if line[3:] != output_rel]
    status = "\n".join(source_status)
    if source_status and not allow_dirty:
        raise SystemExit("source tree has tracked changes; commit sources before making the release artifact")

    package_source_commit = subprocess.run(
        ["git", "-C", str(ROOT), "rev-parse", "HEAD"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()
    source_commit = identity.get("source_commit", "")
    if len(source_commit) != 40 or any(char not in "0123456789abcdef" for char in source_commit):
        raise SystemExit("interactive build identity has an invalid source commit")
    ancestry = subprocess.run(
        ["git", "-C", str(ROOT), "merge-base", "--is-ancestor", source_commit, package_source_commit],
        check=False,
        capture_output=True,
        text=True,
    )
    if ancestry.returncode != 0:
        raise SystemExit("interactive build source commit is not an ancestor of the package source commit")
    image_source_changes = subprocess.run(
        ["git", "-C", str(ROOT), "diff", "--name-only", f"{source_commit}..HEAD"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.splitlines()
    image_relevant_changes = [
        path for path in image_source_changes
        if not path.startswith("docs/virtualbox/") and not path.startswith("tools/virtualbox/")
    ]
    if image_relevant_changes:
        raise SystemExit(
            "interactive build identity is stale; rebuild after changes outside the VirtualBox packaging and documentation paths"
        )
    efi_hash = sha256(EFI)
    esp_hash = sha256(ESP)
    if identity.get("efi_sha256") != efi_hash or identity.get("esp_sha256") != esp_hash:
        raise SystemExit("interactive build identity does not match the EFI and ESP files")
    graphics_profile = (BUILD / "graphics-profile.txt").read_text(encoding="utf-8").strip()
    if graphics_profile != "desktop":
        raise SystemExit("the appliance requires the production desktop graphics profile")
    if identity.get("graphics_profile") != graphics_profile:
        raise SystemExit("interactive build identity has a different graphics profile")
    artifact_id = f"{source_commit}:{efi_hash}:{esp_hash}"
    # A disk UUID is registered globally by VirtualBox. Bind all VM identities
    # to the package source commit so separately published revisions can be
    # opened from different extraction folders without UUID collisions.
    package_identity = f"{artifact_id}:{package_source_commit}"
    package_identity_sha256 = hashlib.sha256(package_identity.encode("utf-8")).hexdigest()
    boot_id = uuid_for("boot-vdi", package_identity)
    data_id = uuid_for("data-vdi", package_identity)
    boot_modification_id = uuid_for("boot-vdi-modify", package_identity)
    data_modification_id = uuid_for("data-vdi-modify", package_identity)
    # Give the VDI package its own VirtualBox registration identity. Earlier
    # releases used the same VM name, settings filename, and machine UUID as
    # the VMDK package, so VirtualBox could keep opening the old registration.
    machine_id = uuid_for("machine-vdi", package_identity)
    machine_name = f"ArenaOS VDI {package_source_commit[:7]}"
    network_mac = "080027" + hashlib.sha256(
        f"arenaos-vbox-mac:{package_identity}".encode("utf-8")
    ).hexdigest()[:6].upper()

    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="arenaos-vbox-") as temp_name:
        temp = Path(temp_name)
        boot_raw = temp / "ArenaOS-Boot.raw"
        data_raw = temp / "ArenaOS-Data.raw"
        make_boot_disk(ESP.read_bytes(), artifact_id, boot_raw)
        make_data_disk(data_raw)
        if data_raw.stat().st_size != DATA_DISK_MIB * 1024 * 1024:
            raise SystemExit("formatted ArenaOS data disk has the wrong size")

        boot_vdi = temp / "ArenaOS-Boot.vdi"
        data_vdi = temp / "ArenaOS-Data.vdi"
        write_vdi(boot_vdi, boot_raw, boot_id, boot_modification_id)
        write_vdi(data_vdi, data_raw, data_id, data_modification_id)
        vbox_path = temp / "ArenaOS-VDI.vbox"
        vbox_path.write_text(
            vbox_xml(machine_id, machine_name, data_id, boot_id, network_mac),
            encoding="utf-8",
            newline="\n",
        )

        manifest = {
            "package_status": "ready_for_external_virtualbox_testing",
            "virtualbox_target_version": VBOX_VERSION,
            "virtualbox_execution_tested": False,
            "source_branch": subprocess.run(
                ["git", "-C", str(ROOT), "branch", "--show-current"],
                check=True,
                capture_output=True,
                text=True,
            ).stdout.strip(),
            "source_commit": source_commit,
            "package_source_commit": package_source_commit,
            "package_identity": package_identity,
            "package_identity_sha256": package_identity_sha256,
            "source_tree_dirty": bool(status),
            "build_identity": identity,
            "graphics_profile": graphics_profile,
            "interactive_efi_sha256": efi_hash,
            "interactive_esp_sha256": esp_hash,
            "phase15_status": "PARTIAL / UNQUALIFIED",
            "boot_disk": {
                "size_bytes": boot_raw.stat().st_size,
                "container_format": "VDI 1.1 fixed",
                "image_uuid": str(boot_id),
                "modification_uuid": str(boot_modification_id),
                "esp_partition_start_lba": ESP_START_LBA,
                "esp_partition_sectors": ESP_SECTORS,
                "disk_sha256": sha256(boot_raw),
                "sata_port": 1,
            },
            "persistent_data_disk": {
                "size_bytes": data_raw.stat().st_size,
                "container_format": "VDI 1.1 fixed",
                "image_uuid": str(data_id),
                "modification_uuid": str(data_modification_id),
                "afs1_size_bytes": AFS1_MIB * 1024 * 1024,
                "afs2_start_sector": AFS2_BASE_SECTOR,
                "afs2_region_initially_blank": True,
                "disk_sha256": sha256(data_raw),
                "sata_port": 0,
            },
            "vm_configuration": {
                "architecture": "x86-64",
                "cpus": 1,
                "memory_mib": 1024,
                "chipset": "ICH9",
                "ioapic": True,
                "firmware": "VirtualBox EFI (provided by installed VirtualBox)",
                "secure_boot": False,
                "display": "VBoxVGA, 16 MiB VRAM, 3D acceleration disabled",
                "input": "PS/2 keyboard and mouse",
                "storage": "AHCI SATA with fixed VDI 1.1 disks; data disk port 0; GPT/FAT EFI disk port 1",
                "network": "VirtualBox paravirtualized VirtIO adapter, NAT",
                "serial_output": "COM1, 38400 8N1, ArenaOS-serial.log",
                "machine_uuid": str(machine_id),
                "machine_name": machine_name,
                "boot_disk_uuid": str(boot_id),
                "data_disk_uuid": str(data_id),
                "network_mac": network_mac,
                "guest_additions": False,
            },
            "package_file_sha256": {},
            "validation_limit": "No VirtualBox guest execution, import, or GUI open was possible in the build environment.",
        }
        manifest_path = temp / "manifest.json"
        manifest_path.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        readme = temp / "README.txt"
        readme.write_text(
            "ArenaOS VirtualBox interactive build\n"
            "======================================\n\n"
            "Requirement: x86-64 Oracle VirtualBox 7.2.22 with host hardware virtualization enabled.\n"
            "Other VirtualBox versions are unverified. No guest additions or host tools are needed.\n\n"
            "1. Extract this ZIP file.\n"
            "2. Open ArenaOS-VDI.vbox in VirtualBox.\n"
            f"3. Select {machine_name} and click Start.\n\n"
            "ArenaOS boot diagnostics are captured automatically in ArenaOS-serial.log beside this file.\n"
            "If the screen stays black or startup stops, power off the VM and send that file.\n\n"
            "Use this new VDI settings file, not an older ArenaOS entry or .vbox file.\n"
            f"If the VM list still shows ArenaOS, open ArenaOS-VDI.vbox from this folder to add {machine_name}.\n\n"
            f"Register one extracted copy of this package. For later starts, select {machine_name} in the VM list\n"
            "instead of opening another extracted copy; VirtualBox registers disk UUIDs globally.\n\n"
            f"Interactive image source commit: {source_commit}\n"
            f"Package generator source commit: {package_source_commit}\n\n"
            "The VM is configured with one CPU, 1024 MiB RAM, ICH9, EFI, VGA, PS/2 input,\n"
            "an AHCI persistent data disk, one VirtIO network adapter using NAT, and\n"
            "COM1 serial output captured in ArenaOS-serial.log beside this file.\n"
            "The disks are VirtualBox native fixed VDI 1.1 images. The first SATA disk is\n"
            "the 72 MiB writable data disk; the second contains the EFI\n"
            "system partition. User files belong on the data disk and persist across boots.\n\n"
            "This package is ready for external VirtualBox testing. Its exact guest has not\n"
            "been started in the packaging environment. Display, input, storage persistence,\n"
            "networking, shutdown, and repeated-boot behavior still require host validation.\n"
            "Phase 15 remains PARTIAL / UNQUALIFIED; this interactive image is not qualification evidence.\n",
            encoding="utf-8",
            newline="\n",
        )

        manifest["package_file_sha256"] = {
            "ArenaOS-VDI.vbox": sha256(vbox_path),
            "ArenaOS-Boot.vdi": sha256(boot_vdi),
            "ArenaOS-Data.vdi": sha256(data_vdi),
            "README.txt": sha256(readme),
        }
        manifest_path.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8")

        with zipfile.ZipFile(output, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
            files = [vbox_path, boot_vdi, data_vdi, manifest_path, readme]
            for file_path in files:
                info = zipfile.ZipInfo(file_path.name, date_time=(2026, 10, 22, 0, 0, 0))
                info.compress_type = zipfile.ZIP_DEFLATED
                info.external_attr = 0o100644 << 16
                archive.writestr(info, file_path.read_bytes(), compress_type=zipfile.ZIP_DEFLATED, compresslevel=9)

    verifier = ROOT / "tools/virtualbox/verify_appliance.py"
    subprocess.run([sys.executable, str(verifier), "--zip", str(output)], check=True)

    print(f"appliance package: {output} ({output.stat().st_size} bytes)")
    print(f"SHA-256: {sha256(output)}")
    print("Note: package structure is generated and checked offline; VirtualBox guest execution remains unverified.")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ARTIFACT)
    parser.add_argument("--allow-dirty", action="store_true", help="for local package development only")
    args = parser.parse_args()
    build(args.output.resolve(), args.allow_dirty)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
