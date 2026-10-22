#!/usr/bin/env python3
"""Independently verify the contents of an ArenaOS VirtualBox package.

This verifier uses only Python's standard library. It checks the package's
file identities, native VirtualBox XML settings, fixed VDI layout, GPT/ESP,
and the AFS1 + blank AFS2 data-disk layout. It does not validate VirtualBox's
ability to open the configuration or execute the guest.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import struct
import tempfile
import uuid
import zipfile
import zlib
from pathlib import Path
from xml.etree import ElementTree

EXPECTED_FILES = {
    "ArenaOS-VDI.vbox",
    "ArenaOS-Boot.vdi",
    "ArenaOS-Data.vdi",
    "Start-ArenaOS.cmd",
    "Start-ArenaOS.sh",
    "README.txt",
    "manifest.json",
}
VBOX_NS = "{http://www.virtualbox.org/}"
SECTOR = 512
MIB = 1024 * 1024
AFS1_SIZE = 8 * MIB
DATA_SIZE = 72 * MIB
ESP_START_LBA = 2048
ESP_SIZE = 16 * MIB
VDI_SIGNATURE = 0xBEDA107F
VDI_VERSION = 0x00010001
VDI_HEADER_SIZE = 400
VDI_BLOCK_SIZE = MIB
VDI_DATA_ALIGNMENT = MIB
VDI_IMAGE_TYPE_FIXED = 2


def fail(message: str) -> None:
    raise SystemExit(f"FAIL: {message}")


def require(condition: bool, message: str) -> None:
    if not condition:
        fail(message)


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def file_hash(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def fat16_file(image: bytes, target: tuple[bytes, ...]) -> bytes:
    """Read a short-name file from a FAT16 ESP without external packages."""
    require(len(image) >= SECTOR and image[510:512] == b"\x55\xaa", "ESP boot sector is invalid")
    bytes_per_sector = struct.unpack_from("<H", image, 11)[0]
    sectors_per_cluster = image[13]
    reserved = struct.unpack_from("<H", image, 14)[0]
    fat_count = image[16]
    root_entries = struct.unpack_from("<H", image, 17)[0]
    total_sectors = struct.unpack_from("<H", image, 19)[0] or struct.unpack_from("<I", image, 32)[0]
    fat_sectors = struct.unpack_from("<H", image, 22)[0]
    require(bytes_per_sector == SECTOR and sectors_per_cluster > 0, "unsupported ESP FAT geometry")
    require(reserved > 0 and fat_count > 0 and fat_sectors > 0, "ESP FAT tables are missing")
    root_sectors = (root_entries * 32 + SECTOR - 1) // SECTOR
    root_start = reserved + fat_count * fat_sectors
    data_start = root_start + root_sectors
    require(data_start < total_sectors <= len(image) // SECTOR, "ESP FAT geometry exceeds partition")
    require(4085 <= total_sectors - data_start < 65525, "ESP is not FAT16")
    fat_start = reserved * SECTOR
    fat = image[fat_start : fat_start + fat_sectors * SECTOR]
    require(len(fat) == fat_sectors * SECTOR, "ESP FAT is truncated")

    def read_cluster(cluster: int) -> bytes:
        require(cluster >= 2, "invalid FAT cluster")
        sector = data_start + (cluster - 2) * sectors_per_cluster
        start = sector * SECTOR
        size = sectors_per_cluster * SECTOR
        require(start + size <= len(image), "FAT cluster exceeds ESP")
        return image[start : start + size]

    def chain(start_cluster: int) -> bytes:
        output = bytearray()
        cluster = start_cluster
        seen: set[int] = set()
        while 2 <= cluster < 0xFFF8:
            require(cluster not in seen, "FAT cluster chain loops")
            seen.add(cluster)
            output.extend(read_cluster(cluster))
            offset = cluster * 2
            require(offset + 2 <= len(fat), "FAT entry exceeds table")
            cluster = struct.unpack_from("<H", fat, offset)[0]
            require(len(seen) <= total_sectors, "FAT cluster chain is unbounded")
        require(cluster >= 0xFFF8, "FAT cluster chain is prematurely terminated")
        return bytes(output)

    def entries(raw: bytes):
        for offset in range(0, len(raw) - 31, 32):
            item = raw[offset : offset + 32]
            if item[0] == 0:
                break
            if item[0] == 0xE5 or item[11] == 0x0F:
                continue
            yield item

    def short_name(part: bytes) -> bytes:
        require(len(part) <= 12 and b"/" not in part, "invalid FAT path component")
        pieces = part.upper().split(b".", 1)
        base = pieces[0].ljust(8, b" ")
        ext = pieces[1].ljust(3, b" ") if len(pieces) == 2 else b" " * 3
        require(len(base) == 8 and len(ext) == 3, "FAT path component is too long")
        return base + ext

    current_entries = list(entries(image[root_start * SECTOR : (root_start + root_sectors) * SECTOR]))
    for index, component in enumerate(target):
        wanted = short_name(component)
        found = next((item for item in current_entries if item[:11] == wanted), None)
        require(found is not None, f"ESP path component {component!r} is missing")
        cluster = struct.unpack_from("<H", found, 26)[0] | (struct.unpack_from("<H", found, 20)[0] << 16)
        is_last = index == len(target) - 1
        if is_last:
            require(found[11] & 0x10 == 0, "EFI image path names a directory")
            size = struct.unpack_from("<I", found, 28)[0]
            data = chain(cluster)[:size]
            require(len(data) == size and size > 0, "EFI image is truncated")
            return data
        require(found[11] & 0x10 != 0, "ESP path component is not a directory")
        current_entries = list(entries(chain(cluster)))
    fail("empty ESP path")


def verify_gpt(boot: bytes, manifest: dict) -> bytes:
    require(len(boot) % SECTOR == 0 and len(boot) == manifest["boot_disk"]["size_bytes"], "boot disk size mismatch")
    require(boot[510:512] == b"\x55\xaa", "protective MBR signature missing")
    mbr = boot[446:462]
    require(mbr[4] == 0xEE and struct.unpack_from("<I", mbr, 8)[0] == 1, "protective MBR entry invalid")
    total_lba = len(boot) // SECTOR
    primary = boot[SECTOR : 2 * SECTOR]
    backup = boot[(total_lba - 1) * SECTOR : total_lba * SECTOR]

    def check_header(raw: bytes, current: int, backup_lba: int, table_lba: int) -> bytes:
        require(raw[:8] == b"EFI PART", "GPT signature missing")
        revision, header_size, crc = struct.unpack_from("<III", raw, 8)
        require(revision == 0x00010000 and header_size == 92, "unsupported GPT header")
        header = bytearray(raw[:header_size])
        struct.pack_into("<I", header, 16, 0)
        require(zlib.crc32(header) & 0xFFFFFFFF == crc, "GPT header checksum mismatch")
        current_lba, backup_value = struct.unpack_from("<QQ", raw, 24)
        first_usable, last_usable = struct.unpack_from("<QQ", raw, 40)
        part_table_lba = struct.unpack_from("<Q", raw, 72)[0]
        count, entry_size, table_crc = struct.unpack_from("<III", raw, 80)
        require((current_lba, backup_value, part_table_lba) == (current, backup_lba, table_lba), "GPT header locations mismatch")
        require(first_usable == 34 and last_usable == total_lba - 34, "GPT usable range mismatch")
        require(count == 128 and entry_size == 128, "GPT entry geometry mismatch")
        table = boot[table_lba * SECTOR : table_lba * SECTOR + count * entry_size]
        require(len(table) == count * entry_size and zlib.crc32(table) & 0xFFFFFFFF == table_crc, "GPT table checksum mismatch")
        return table

    primary_table = check_header(primary, 1, total_lba - 1, 2)
    backup_table_lba = total_lba - 33
    backup_table = check_header(backup, total_lba - 1, 1, backup_table_lba)
    require(primary_table == backup_table, "primary and backup GPT tables differ")
    entry = primary_table[:128]
    require(entry[:16] == uuid.UUID("c12a7328-f81f-11d2-ba4b-00a0c93ec93b").bytes_le, "ESP partition type GUID mismatch")
    start, end = struct.unpack_from("<QQ", entry, 32)
    require(start == ESP_START_LBA and end - start + 1 == ESP_SIZE // SECTOR, "ESP partition geometry mismatch")
    esp = boot[start * SECTOR : (end + 1) * SECTOR]
    require(sha256(esp) == manifest["interactive_esp_sha256"], "embedded ESP SHA-256 mismatch")
    efi = fat16_file(esp, (b"EFI", b"BOOT", b"BOOTX64.EFI"))
    require(sha256(efi) == manifest["interactive_efi_sha256"], "embedded EFI SHA-256 mismatch")
    return esp


def verify_afs1_and_afs2(data: bytes) -> None:
    require(len(data) == DATA_SIZE, "persistent data disk must be 72 MiB")
    legacy = data[:AFS1_SIZE]
    sector = legacy[:SECTOR]
    require(sector[:8] == b"ARENAFS1", "AFS1 superblock missing")
    fnv = 0xCBF29CE484222325
    for value in sector[:504]:
        fnv = ((fnv ^ value) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    require(struct.unpack_from("<Q", sector, 504)[0] == fnv, "AFS1 superblock checksum mismatch")
    fields = struct.unpack_from("<8sIIIIIII", sector, 0)
    require(fields == (b"ARENAFS1", 1, SECTOR, AFS1_SIZE // SECTOR, 32, 64, 4, 4), "AFS1 geometry mismatch")
    require(struct.unpack_from("<I", sector, 36)[0] == 1, "AFS1 commit base mismatch")
    valid_commit = False
    for slot in (1, 2):
        raw = legacy[slot * SECTOR : (slot + 1) * SECTOR]
        if struct.unpack_from("<II", raw, 0) == (0x43534641, 1):
            checksum = 0xCBF29CE484222325
            for value in raw[:504]:
                checksum = ((checksum ^ value) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
            if struct.unpack_from("<Q", raw, 504)[0] == checksum:
                valid_commit = True
    require(valid_commit, "AFS1 has no valid commit record")
    require(not any(data[AFS1_SIZE:]), "initial AFS2 region must be blank for first-boot import")


def verify_vdi(path: Path, expected_size: int) -> tuple[bytes, uuid.UUID, uuid.UUID]:
    """Validate a fixed VDI 1.1 image and return raw data and both UUIDs."""
    image = path.read_bytes()
    info = b"<<< Oracle VirtualBox Disk Image >>>\n".ljust(64, b"\0")
    require(image[:64] == info, f"{path.name} VDI file info mismatch")
    signature, version = struct.unpack_from("<II", image, 64)
    require(signature == VDI_SIGNATURE and version == VDI_VERSION, f"{path.name} VDI signature/version mismatch")
    require(len(image) >= 72 + VDI_HEADER_SIZE, f"{path.name} VDI header is truncated")
    header_size, image_type, flags = struct.unpack_from("<III", image, 72)
    require(header_size == VDI_HEADER_SIZE, f"{path.name} VDI header size mismatch")
    require(image_type == VDI_IMAGE_TYPE_FIXED and flags == 0, f"{path.name} is not a normal fixed VDI image")
    off_blocks, off_data = struct.unpack_from("<II", image, 72 + 268)
    disk_size, block_size, block_extra, block_count, allocated_count = struct.unpack_from("<QIIII", image, 72 + 296)
    require(disk_size == expected_size and disk_size % SECTOR == 0, f"{path.name} logical disk size mismatch")
    require(block_size == VDI_BLOCK_SIZE and block_extra == 0, f"{path.name} VDI block geometry mismatch")
    expected_blocks = (disk_size + block_size - 1) // block_size
    require(block_count == expected_blocks and allocated_count == block_count, f"{path.name} VDI allocation count mismatch")
    require(off_blocks % VDI_DATA_ALIGNMENT == 0 and off_data % VDI_DATA_ALIGNMENT == 0, f"{path.name} VDI offsets are not aligned")
    require(off_blocks >= 72 + VDI_HEADER_SIZE, f"{path.name} block map overlaps its header")
    require(off_data >= off_blocks + block_count * 4, f"{path.name} data overlaps its block map")
    require(len(image) == off_data + disk_size, f"{path.name} VDI file length mismatch")
    require(image[off_blocks : off_blocks + block_count * 4] == struct.pack(f"<{block_count}I", *range(block_count)), f"{path.name} fixed VDI block map is invalid")
    legacy_sector_size = struct.unpack_from("<I", image, 72 + 276 + 12)[0]
    lchs_sector_size = struct.unpack_from("<I", image, 72 + 384 + 12)[0]
    require(legacy_sector_size == SECTOR and lchs_sector_size == SECTOR, f"{path.name} VDI sector geometry mismatch")
    image_uuid = uuid.UUID(bytes_le=image[72 + 320 : 72 + 336])
    modification_uuid = uuid.UUID(bytes_le=image[72 + 336 : 72 + 352])
    require(image_uuid.int != 0, f"{path.name} VDI creation UUID is zero")
    require(modification_uuid.int != 0, f"{path.name} VDI modification UUID is zero")
    return image[off_data:], image_uuid, modification_uuid


def verify_xml(path: Path, vm_configuration: dict) -> dict[str, uuid.UUID]:
    root = ElementTree.parse(path).getroot()
    require(root.tag == VBOX_NS + "VirtualBox", "VirtualBox XML root namespace mismatch")
    require(root.attrib.get("version") == "1.20-linux", "VirtualBox settings version mismatch")
    machine = root.find(VBOX_NS + "Machine")
    require(machine is not None and machine.attrib.get("name") == vm_configuration.get("machine_name") and machine.attrib.get("OSType") == "Other_64", "machine identity mismatch")
    require(uuid.UUID(machine.attrib["uuid"].strip("{}")) == uuid.UUID(vm_configuration["machine_uuid"]), "machine UUID does not match manifest")
    platform = machine.find(VBOX_NS + "Platform")
    require(platform is not None, "machine Platform element is missing")
    require(platform.attrib.get("architecture") == "x86", "machine platform architecture mismatch")
    platform_cpu = platform.find(VBOX_NS + "CPU")
    require(platform_cpu is not None and platform_cpu.attrib.get("count") == "1" and platform_cpu.attrib.get("hotplug") == "false", "VM platform CPU settings mismatch")
    chipset = platform.find(VBOX_NS + "Chipset")
    require(chipset is not None and chipset.attrib.get("type") == "ICH9", "VM chipset mismatch")
    rtc = platform.find(VBOX_NS + "RTC")
    require(rtc is not None and rtc.attrib.get("localOrUTC") == "UTC", "VM RTC settings mismatch")
    x86 = platform.find(VBOX_NS + "x86")
    require(x86 is not None, "machine x86 platform element is missing")
    x86_cpu = x86.find(VBOX_NS + "CPU")
    require(x86_cpu is not None, "machine x86 CPU settings are missing")
    for name, expected in {
        "PAE": "false",
        "LongMode": "true",
        "X2APIC": "false",
        "HardwareVirtExLargePages": "false",
        "HardwareVirtExNestedPaging": "true",
        "HardwareVirtExVPID": "true",
        "HardwareVirtExUX": "true",
    }.items():
        setting = x86_cpu.find(VBOX_NS + name)
        require(setting is not None and setting.attrib.get("enabled") == expected, f"VM x86 {name} setting mismatch")

    hardware = machine.find(VBOX_NS + "Hardware")
    require(hardware is not None, "machine hardware block is missing")
    memory = hardware.find(VBOX_NS + "Memory")
    firmware = hardware.find(VBOX_NS + "Firmware")
    display = hardware.find(VBOX_NS + "Display")
    hid = hardware.find(VBOX_NS + "HID")
    require(hardware.find(VBOX_NS + "CPU") is None, "legacy Hardware/CPU settings must be stored under Platform")
    require(memory is not None and memory.attrib.get("RAMSize") == "1024", "VM must have 1024 MiB RAM")
    require(hardware.find(VBOX_NS + "Chipset") is None, "legacy Hardware/Chipset setting must be stored under Platform")
    require(firmware is not None and firmware.attrib.get("type") == "EFI" and firmware.attrib.get("secureBootEnabled") == "false", "VM EFI settings mismatch")
    require(hardware.find(VBOX_NS + "BIOS") is None, "settings version 1.20 must not use the legacy Hardware/BIOS node")
    acpi = firmware.find(VBOX_NS + "ACPI")
    require(acpi is not None and acpi.attrib.get("enabled") == "true", "VM ACPI setting mismatch")
    ioapic = firmware.find(VBOX_NS + "IOAPIC")
    require(ioapic is not None and ioapic.attrib.get("enabled") == "true", "VM I/O APIC is disabled in the settings-version 1.20 Firmware node")
    require(display is not None and display.attrib.get("controller") == "VBoxVGA", "VM display controller mismatch")
    require(hid is not None and hid.attrib.get("Keyboard") == "PS2Keyboard" and hid.attrib.get("Pointing") == "PS2Mouse", "VM input devices mismatch")

    boot = hardware.find(VBOX_NS + "Boot")
    require(boot is not None and boot.find(VBOX_NS + "Order").attrib.get("device") == "HardDisk", "VM must boot from a hard disk")
    network = hardware.find(VBOX_NS + "Network")
    adapter = network.find(VBOX_NS + "Adapter") if network is not None else None
    require(adapter is not None and adapter.attrib.get("enabled") == "true" and adapter.attrib.get("type") == "virtio", "VM network adapter mismatch")
    require(adapter.attrib.get("MACAddress") == vm_configuration.get("network_mac"), "VM network MAC does not match manifest")
    require(adapter.find(VBOX_NS + "NAT") is not None, "VM network adapter is not using NAT")
    uart = hardware.find(VBOX_NS + "UART")
    serial_ports = uart.findall(VBOX_NS + "Port") if uart is not None else []
    require(
        len(serial_ports) == 1
        and serial_ports[0].attrib.get("slot") == "0"
        and serial_ports[0].attrib.get("enabled") == "true"
        and serial_ports[0].attrib.get("IOAddress") == "0x3f8"
        and serial_ports[0].attrib.get("IRQ") == "4"
        and serial_ports[0].attrib.get("hostMode") == "RawFile"
        and serial_ports[0].attrib.get("path") == "ArenaOS-serial.log"
        and vm_configuration.get("serial_output") == "COM1, 38400 8N1, absolute package-local ArenaOS-serial.log set by launcher",
        "VM COM1 raw-file diagnostic output mismatch",
    )

    registry = machine.find(VBOX_NS + "MediaRegistry")
    hard_disks = registry.find(VBOX_NS + "HardDisks") if registry is not None else None
    disks = hard_disks.findall(VBOX_NS + "HardDisk") if hard_disks is not None else []
    by_name = {Path(item.attrib["location"]).name: item for item in disks}
    require(set(by_name) == {"ArenaOS-Data.vdi", "ArenaOS-Boot.vdi"}, "registered disk set mismatch")
    require(all(item.attrib.get("format") == "VDI" for item in by_name.values()), "registered disks must use native VDI format")
    storage = hardware.find(VBOX_NS + "StorageControllers")
    controller = storage.find(VBOX_NS + "StorageController") if storage is not None else None
    require(controller is not None and controller.attrib.get("type") == "AHCI" and controller.attrib.get("PortCount") == "2", "VM AHCI controller mismatch")
    attached = controller.findall(VBOX_NS + "AttachedDevice")
    ports = {item.attrib.get("port"): item.find(VBOX_NS + "Image").attrib.get("uuid") for item in attached}
    require(ports == {"0": by_name["ArenaOS-Data.vdi"].attrib["uuid"], "1": by_name["ArenaOS-Boot.vdi"].attrib["uuid"]}, "AHCI disk port assignments mismatch")
    return {
        name: uuid.UUID(item.attrib["uuid"].strip("{}"))
        for name, item in by_name.items()
    }


def verify_directory(root: Path) -> None:
    require(root.is_dir(), "package directory does not exist")
    require(all(path.is_file() for path in root.iterdir()), "package has unexpected nested or non-file entries")
    actual = {path.name for path in root.iterdir() if path.is_file()}
    require(actual == EXPECTED_FILES, f"package file set mismatch: {sorted(actual)}")
    manifest = json.loads((root / "manifest.json").read_text(encoding="utf-8"))
    require(manifest.get("package_status") == "ready_for_external_virtualbox_testing", "package status is not external-test-ready")
    require(manifest.get("phase15_status") == "PARTIAL / UNQUALIFIED", "Phase 15 status was changed")
    require(manifest.get("virtualbox_execution_tested") is False, "manifest claims unrecorded VirtualBox execution")
    require(manifest.get("virtualbox_target_version") == "7.2.22", "VirtualBox target version mismatch")
    require(manifest.get("graphics_profile") == "desktop", "interactive desktop graphics profile mismatch")
    require(re.fullmatch(r"[0-9a-f]{40}", manifest.get("source_commit", "")) is not None, "source commit identity invalid")
    require(re.fullmatch(r"[0-9a-f]{40}", manifest.get("package_source_commit", "")) is not None, "package source commit identity invalid")
    package_identity = ":".join((
        manifest["source_commit"],
        manifest["interactive_efi_sha256"],
        manifest["interactive_esp_sha256"],
        manifest["package_source_commit"],
    ))
    require(manifest.get("package_identity") == package_identity, "package identity does not match source/artifact commits")
    require(
        manifest.get("package_identity_sha256") == hashlib.sha256(package_identity.encode("utf-8")).hexdigest(),
        "package identity hash mismatch",
    )
    expected_uuids = {
        name: uuid.uuid5(uuid.NAMESPACE_URL, f"https://arenaos.invalid/{name}/{package_identity}")
        for name in (
            "machine-vdi",
            "boot-vdi",
            "data-vdi",
            "boot-vdi-modify",
            "data-vdi-modify",
        )
    }
    vm_configuration = manifest.get("vm_configuration") or {}
    require(
        vm_configuration.get("machine_name") == f"ArenaOS VDI {manifest['package_source_commit'][:7]}",
        "machine name is not unique to this package revision",
    )
    require(vm_configuration.get("machine_uuid") == str(expected_uuids["machine-vdi"]), "machine UUID is not unique to this package identity")
    require(vm_configuration.get("boot_disk_uuid") == str(expected_uuids["boot-vdi"]), "boot disk UUID is not unique to this package identity")
    require(vm_configuration.get("data_disk_uuid") == str(expected_uuids["data-vdi"]), "data disk UUID is not unique to this package identity")
    expected_mac = "080027" + hashlib.sha256(f"arenaos-vbox-mac:{package_identity}".encode("utf-8")).hexdigest()[:6].upper()
    require(vm_configuration.get("network_mac") == expected_mac, "network MAC is not unique to this package identity")
    build_identity = manifest.get("build_identity") or {}
    require(build_identity.get("source_commit") == manifest.get("source_commit"), "interactive image source commit mismatch")
    require(build_identity.get("efi_sha256") == manifest.get("interactive_efi_sha256"), "interactive EFI build identity mismatch")
    require(build_identity.get("esp_sha256") == manifest.get("interactive_esp_sha256"), "interactive ESP build identity mismatch")
    hashes = manifest.get("package_file_sha256")
    require(set(hashes or {}) == EXPECTED_FILES - {"manifest.json"}, "package manifest hash set mismatch")
    for name, digest in hashes.items():
        require(file_hash(root / name) == digest, f"package file SHA-256 mismatch: {name}")

    require(manifest["boot_disk"].get("container_format") == "VDI 1.1 fixed", "boot disk format mismatch")
    require(manifest["persistent_data_disk"].get("container_format") == "VDI 1.1 fixed", "persistent data disk format mismatch")
    boot, boot_uuid, boot_modification_uuid = verify_vdi(root / "ArenaOS-Boot.vdi", manifest["boot_disk"]["size_bytes"])
    data, data_uuid, data_modification_uuid = verify_vdi(root / "ArenaOS-Data.vdi", manifest["persistent_data_disk"]["size_bytes"])
    require(sha256(boot) == manifest["boot_disk"]["disk_sha256"], "boot logical disk identity mismatch")
    require(sha256(data) == manifest["persistent_data_disk"]["disk_sha256"], "data logical disk identity mismatch")
    require(boot_uuid == expected_uuids["boot-vdi"], "boot VDI creation UUID is not unique to this package identity")
    require(data_uuid == expected_uuids["data-vdi"], "data VDI creation UUID is not unique to this package identity")
    require(boot_modification_uuid == expected_uuids["boot-vdi-modify"], "boot VDI modification UUID mismatch")
    require(data_modification_uuid == expected_uuids["data-vdi-modify"], "data VDI modification UUID mismatch")
    require(str(boot_uuid) == manifest["boot_disk"]["image_uuid"], "boot VDI creation UUID manifest mismatch")
    require(str(data_uuid) == manifest["persistent_data_disk"]["image_uuid"], "data VDI creation UUID manifest mismatch")
    require(str(boot_modification_uuid) == manifest["boot_disk"]["modification_uuid"], "boot VDI modification UUID manifest mismatch")
    require(str(data_modification_uuid) == manifest["persistent_data_disk"]["modification_uuid"], "data VDI modification UUID manifest mismatch")
    xml_uuids = verify_xml(root / "ArenaOS-VDI.vbox", vm_configuration)
    require(xml_uuids["ArenaOS-Boot.vdi"] == boot_uuid, "boot VDI UUID does not match ArenaOS-VDI.vbox")
    require(xml_uuids["ArenaOS-Data.vdi"] == data_uuid, "data VDI UUID does not match ArenaOS-VDI.vbox")
    verify_gpt(boot, manifest)
    verify_afs1_and_afs2(data)
    windows_launcher = (root / "Start-ArenaOS.cmd").read_text(encoding="utf-8")
    linux_launcher = (root / "Start-ArenaOS.sh").read_text(encoding="utf-8")
    vm_uuid = vm_configuration["machine_uuid"]
    require(
        vm_uuid in windows_launcher
        and "--uartmode1 file \"%~dp0ArenaOS-serial.log\"" in windows_launcher
        and "registervm \"%~dp0ArenaOS-VDI.vbox\"" in windows_launcher
        and "startvm \"%VM_UUID%\" --type gui" in windows_launcher,
        "Windows launcher does not register, pin the absolute serial path, and start the expected VM",
    )
    require(
        f"VM_UUID='{{{vm_uuid}}}'" in linux_launcher
        and '--uartmode1 file "$SCRIPT_DIR/ArenaOS-serial.log"' in linux_launcher
        and 'registervm "$SCRIPT_DIR/ArenaOS-VDI.vbox"' in linux_launcher
        and 'startvm "$VM_UUID" --type gui' in linux_launcher,
        "Linux launcher does not register, pin the absolute serial path, and start the expected VM",
    )
    readme = (root / "README.txt").read_text(encoding="utf-8")
    require(
        "double-click Start-ArenaOS.cmd" in readme
        and "sh Start-ArenaOS.sh" in readme
        and f"The launcher targets {vm_configuration['machine_name']}" in readme
        and "VirtualBox registers disk UUIDs globally" in readme
        and "ArenaOS-serial.log" in readme
        and "send ArenaOS-serial.log" in readme
        and "Phase 15 remains PARTIAL / UNQUALIFIED" in readme,
        "README instructions or Phase 15 status missing",
    )
    print("PASS: package identities, relative VM configuration, fixed VDI images, GPT/EFI, AFS1, and blank AFS2 region")
    print(f"source commit: {manifest['source_commit']}")
    print(f"EFI SHA-256: {manifest['interactive_efi_sha256']}")
    print(f"ESP SHA-256: {manifest['interactive_esp_sha256']}")
    print("VirtualBox open/import and guest execution: NOT TESTED")


def verify_zip(path: Path) -> None:
    require(path.is_file(), "package ZIP does not exist")
    with zipfile.ZipFile(path) as archive:
        members = archive.namelist()
        require(len(members) == len(set(members)), "ZIP contains duplicate member names")
        require(set(members) == EXPECTED_FILES, f"ZIP member set mismatch: {sorted(members)}")
        for member in members:
            require(Path(member).name == member and not member.startswith(("/", "\\")) and ".." not in Path(member).parts, f"unsafe ZIP member path: {member}")
        with tempfile.TemporaryDirectory(prefix="arenaos-vbox-verify-") as temporary:
            archive.extractall(temporary)
            verify_directory(Path(temporary))
    print(f"package ZIP SHA-256: {file_hash(path)}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--zip", type=Path, help="validate and freshly extract this ZIP")
    group.add_argument("--directory", type=Path, help="validate a fresh extraction directory")
    args = parser.parse_args()
    if args.zip:
        verify_zip(args.zip.resolve())
    else:
        verify_directory(args.directory.resolve())
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
