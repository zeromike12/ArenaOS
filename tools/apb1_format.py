#!/usr/bin/env python3
"""Independent host reference codec and test-only APB1 fixture builder.

This is not the ArenaOS receiver implementation. It intentionally uses
Python stdlib and OpenSSL, and embeds only the publicly known RFC 8032 vector-1
private seed for signed test fixtures. The seed is written only to an
OpenSSL temporary directory and must never be used for a release package.
"""
from __future__ import annotations

import hashlib
import subprocess
import tempfile
from dataclasses import dataclass
from pathlib import Path

DOMAIN = b"ArenaOS.application-bundle.v1\0"
MAGIC = b"APB1"
HEADER_BYTES = 64
MANIFEST_BYTES = 512
RECORD_BYTES = 144
MAX_FILES = 64
MAX_FILE_BYTES = 16 * 1024 * 1024
MAX_TOTAL_BYTES = 32 * 1024 * 1024
MAX_METADATA_BYTES = HEADER_BYTES + MANIFEST_BYTES + MAX_FILES * RECORD_BYTES
MAX_SIGNED_BYTES = len(DOMAIN) + MAX_METADATA_BYTES
CHUNK_BYTES = 4096
# Phase 13 adds signed stream/synchronization request hints in bits 3 and 4.
# These bits remain descriptive; launch policy mints the actual capabilities.
KNOWN_APP_FLAGS = 0x1F
RFC_SEED = bytes.fromhex(
    "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60"
)
RFC_PUBLIC = bytes.fromhex(
    "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"
)
PKCS8 = bytes.fromhex("302e020100300506032b657004220420")
SPKI = bytes.fromhex("302a300506032b6570032100")


class Refusal(ValueError):
    """Fail-closed APB1 reference parser refusal."""


def _fixed(text: bytes, width: int) -> bytes:
    if not text or len(text) >= width or b"\0" in text:
        raise ValueError("bad fixed-width field")
    return text + bytes(width - len(text))


def _valid_path(path: bytes) -> bool:
    if not path or len(path) > 95 or path.startswith(b"/") or path.endswith(b"/"):
        return False
    if any(b < 0x21 or b > 0x7e or b in (ord("\\"), ord(":")) for b in path):
        return False
    return all(c not in (b"", b".", b"..") for c in path.split(b"/"))


def _valid_content_type(value: bytes) -> bool:
    allowed = b"abcdefghijklmnopqrstuvwxyz0123456789/!#$&^_.+-"
    return (
        value.count(b"/") == 1
        and value.split(b"/", 1)[0]
        and value.split(b"/", 1)[1]
        and all(ch in allowed for ch in value)
    )


def make_manifest(*, app_id: bytes = b"com.arena.editor",
                  package_id: bytes = b"org.arena.editor",
                  display_name: bytes = b"Text Editor", version: int = 42,
                  flags: int = 1, requested: int = 1,
                  entry: bytes = b"bin/editor",
                  icon: bytes = b"icons/editor.bin",
                  width: int = 640, height: int = 480,
                  associations: tuple[bytes, ...] = (b"text/markdown", b"text/plain")) -> bytes:
    out = bytearray(MANIFEST_BYTES)
    out[0:4] = b"AMF1"
    out[4:6] = (1).to_bytes(2, "little")
    out[6:8] = MANIFEST_BYTES.to_bytes(2, "little")
    out[8:40] = _fixed(app_id, 32)
    out[40:72] = _fixed(package_id, 32)
    out[72:104] = _fixed(display_name, 32)
    out[104:112] = version.to_bytes(8, "little")
    out[112:116] = flags.to_bytes(4, "little")
    out[116:120] = requested.to_bytes(4, "little")
    out[120:184] = _fixed(entry, 64)
    out[184:248] = bytes(64) if not icon else _fixed(icon, 64)
    out[248:250] = width.to_bytes(2, "little")
    out[250:252] = height.to_bytes(2, "little")
    out[252:256] = len(associations).to_bytes(4, "little")
    if len(associations) > 8 or tuple(sorted(set(associations))) != associations:
        raise ValueError("association list must be sorted, unique, and <= 8")
    for i, content_type in enumerate(associations):
        if not _valid_content_type(content_type):
            raise ValueError("invalid content type")
        out[256 + 32*i:288 + 32*i] = _fixed(content_type, 32)
    return bytes(out)


def make_record(kind: int, path: bytes, payload: bytes) -> bytes:
    # Permit syntactically sized but semantically hostile paths here so the
    # negative signed fixtures exercise receiver-side traversal checks.
    if kind not in (1, 2) or not 0 < len(path) <= 95 or b"\\0" in path or not 0 < len(payload) <= MAX_FILE_BYTES:
        raise ValueError("invalid file record")
    return (
        bytes([kind]) + len(path).to_bytes(2, "little") + bytes(5)
        + len(payload).to_bytes(8, "little") + hashlib.sha256(payload).digest()
        + _fixed(path, 96)
    )


def sign(seed: bytes, message: bytes) -> bytes:
    with tempfile.TemporaryDirectory(prefix="arena-apb1-test-") as td:
        p = Path(td)
        (p / "key.der").write_bytes(PKCS8 + seed)
        (p / "message").write_bytes(message)
        subprocess.run(
            ["openssl", "pkeyutl", "-sign", "-rawin", "-keyform", "DER",
             "-inkey", str(p / "key.der"), "-in", str(p / "message"),
             "-out", str(p / "signature")],
            check=True, capture_output=True,
        )
        signature = (p / "signature").read_bytes()
    if len(signature) != 64:
        raise RuntimeError("OpenSSL returned a non-Ed25519 signature")
    return signature


def verify_openssl(public_key: bytes, message: bytes, signature: bytes) -> bool:
    if len(public_key) != 32 or len(signature) != 64:
        return False
    with tempfile.TemporaryDirectory(prefix="arena-apb1-test-") as td:
        p = Path(td)
        (p / "key.der").write_bytes(SPKI + public_key)
        (p / "message").write_bytes(message)
        (p / "signature").write_bytes(signature)
        result = subprocess.run(
            ["openssl", "pkeyutl", "-verify", "-rawin", "-pubin", "-keyform", "DER",
             "-inkey", str(p / "key.der"), "-in", str(p / "message"),
             "-sigfile", str(p / "signature")], capture_output=True,
        )
        return result.returncode == 0


def build_bundle(files: list[tuple[int, bytes, bytes]], *, manifest: bytes | None = None,
                 seed: bytes = RFC_SEED) -> bytes:
    if not 1 <= len(files) <= MAX_FILES:
        raise ValueError("file count out of range")
    if manifest is None:
        manifest = make_manifest()
    if len(manifest) != MANIFEST_BYTES:
        raise ValueError("manifest length")
    ordered = sorted(files, key=lambda f: f[1])
    if files != ordered:
        raise ValueError("fixture records must be sorted")
    table = b"".join(make_record(kind, path, data) for kind, path, data in files)
    payload = b"".join(data for _, _, data in files)
    if not 0 < len(payload) <= MAX_TOTAL_BYTES:
        raise ValueError("payload total out of range")
    public_key = RFC_PUBLIC if seed == RFC_SEED else _public_from_seed(seed)
    header = bytearray(HEADER_BYTES)
    header[0:4] = MAGIC
    header[4:6] = (1).to_bytes(2, "little")
    header[6:8] = HEADER_BYTES.to_bytes(2, "little")
    header[8:12] = MANIFEST_BYTES.to_bytes(4, "little")
    header[12:16] = len(files).to_bytes(4, "little")
    header[16:24] = len(table).to_bytes(8, "little")
    header[24:32] = len(payload).to_bytes(8, "little")
    header[32:64] = hashlib.sha256(public_key).digest()
    metadata = bytes(header) + manifest + table
    signature = sign(seed, DOMAIN + metadata)
    return metadata + signature + payload


def _public_from_seed(seed: bytes) -> bytes:
    with tempfile.TemporaryDirectory(prefix="arena-apb1-test-") as td:
        p = Path(td)
        (p / "key.der").write_bytes(PKCS8 + seed)
        subprocess.run(["openssl", "pkey", "-inform", "DER", "-in", str(p / "key.der"),
                        "-pubout", "-outform", "DER", "-out", str(p / "pub.der")],
                       check=True, capture_output=True)
        der = (p / "pub.der").read_bytes()
    if len(der) != len(SPKI) + 32 or der[:len(SPKI)] != SPKI:
        raise RuntimeError("unexpected Ed25519 SPKI")
    return der[len(SPKI):]


@dataclass(frozen=True)
class ParsedBundle:
    metadata: bytes
    signature: bytes
    payload: bytes
    files: tuple[tuple[int, bytes, int, bytes], ...]
    payload_offset: int
    bundle_digest: bytes


def _parse_manifest(data: bytes) -> tuple[bytes, bytes, bytes, int, int, int, bytes, bytes]:
    if len(data) != MANIFEST_BYTES or data[:4] != b"AMF1":
        raise Refusal("manifest header")
    if int.from_bytes(data[4:6], "little") != 1 or int.from_bytes(data[6:8], "little") != MANIFEST_BYTES:
        raise Refusal("manifest version/length")
    app_id, package_id, label = data[8:40], data[40:72], data[72:104]
    for ident in (app_id, package_id):
        end = ident.find(b"\0")
        if not 1 <= end <= 31 or ident[end:] != bytes(32-end):
            raise Refusal("identity padding")
        if any(c not in b"abcdefghijklmnopqrstuvwxyz0123456789.-" for c in ident[:end]):
            raise Refusal("identity alphabet")
        if not (ident[0:1].islower() or ident[0:1].isdigit()):
            raise Refusal("identity first byte")
    end = label.find(b"\0")
    if not 1 <= end <= 31 or label[end:] != bytes(32-end) or any(c < 32 or c > 126 for c in label[:end]):
        raise Refusal("display name")
    version = int.from_bytes(data[104:112], "little")
    flags = int.from_bytes(data[112:116], "little")
    requests = int.from_bytes(data[116:120], "little")
    entry_field, icon_field = data[120:184], data[184:248]
    width, height = int.from_bytes(data[248:250], "little"), int.from_bytes(data[250:252], "little")
    assoc_count = int.from_bytes(data[252:256], "little")
    if version == 0 or flags & ~KNOWN_APP_FLAGS or requests & ~15:
        raise Refusal("reserved manifest bits")
    if (width, height) == (0, 0):
        if not flags & 4:
            raise Refusal("headless dimensions")
    elif not 80 <= width <= 1024 or not 60 <= height <= 768:
        raise Refusal("window dimensions")
    if assoc_count > 8:
        raise Refusal("association count")
    assoc = []
    for i in range(8):
        field = data[256+32*i:288+32*i]
        end = field.find(b"\0")
        if i < assoc_count:
            if end <= 0 or field[end:] != bytes(32-end) or not _valid_content_type(field[:end]):
                raise Refusal("content type")
            assoc.append(field[:end])
        elif field != bytes(32):
            raise Refusal("association padding")
    if assoc != sorted(set(assoc)):
        raise Refusal("association order")
    for field in (entry_field, icon_field):
        end = field.find(b"\0")
        if end < 0 or field[end:] != bytes(64-end):
            raise Refusal("path padding")
        if end == 0:
            if field is entry_field:
                raise Refusal("missing entry path")
        elif not _valid_path(field[:end]):
            raise Refusal("manifest path")
    if entry_field[:entry_field.find(b"\0")] == icon_field[:icon_field.find(b"\0")]:
        raise Refusal("entry/icon alias")
    return app_id, package_id, entry_field, flags, requests, version, icon_field, label


def parse_bundle(data: bytes) -> ParsedBundle:
    if len(data) < HEADER_BYTES + MANIFEST_BYTES + RECORD_BYTES + 64:
        raise Refusal("short bundle")
    header = data[:HEADER_BYTES]
    if header[:4] != MAGIC:
        raise Refusal("magic")
    if int.from_bytes(header[4:6], "little") != 1 or int.from_bytes(header[6:8], "little") != HEADER_BYTES:
        raise Refusal("version/header length")
    manifest_len = int.from_bytes(header[8:12], "little")
    count = int.from_bytes(header[12:16], "little")
    table_len = int.from_bytes(header[16:24], "little")
    payload_len = int.from_bytes(header[24:32], "little")
    if manifest_len != MANIFEST_BYTES or not 1 <= count <= MAX_FILES:
        raise Refusal("manifest/file count")
    if table_len != count * RECORD_BYTES or payload_len == 0 or payload_len > MAX_TOTAL_BYTES:
        raise Refusal("table/payload bounds")
    metadata_len = HEADER_BYTES + manifest_len + table_len
    if metadata_len > MAX_METADATA_BYTES or metadata_len + 64 + payload_len != len(data):
        raise Refusal("exact envelope")
    metadata = data[:metadata_len]
    manifest = _parse_manifest(metadata[HEADER_BYTES:HEADER_BYTES+MANIFEST_BYTES])
    app_id, package_id, entry_field, _, _, _, icon_field, _ = manifest
    signed_entry = entry_field[:entry_field.find(b"\0")]
    icon_end = icon_field.find(b"\0")
    signed_icon = icon_field[:icon_end] if icon_end > 0 else None
    records = []
    paths = []
    total = 0
    exec_count = 0
    has_entry = False
    has_icon = signed_icon is None
    payload_at = metadata_len + 64
    payload_cursor = payload_at
    for i in range(count):
        start = HEADER_BYTES + MANIFEST_BYTES + i * RECORD_BYTES
        record = metadata[start:start+RECORD_BYTES]
        kind = record[0]
        path_len = int.from_bytes(record[1:3], "little")
        if kind not in (1, 2) or not 1 <= path_len <= 95 or record[3:8] != bytes(5):
            raise Refusal("record kind/length/reserved")
        size = int.from_bytes(record[8:16], "little")
        digest = record[16:48]
        path_field = record[48:144]
        path = path_field[:path_len]
        if path_field[path_len:] != bytes(96-path_len) or not _valid_path(path):
            raise Refusal("record path")
        if path == b"APB1.record":
            raise Refusal("reserved install metadata path")
        if paths and paths[-1] >= path:
            raise Refusal("record ordering/duplicate")
        if paths and path.startswith(paths[-1] + b"/"):
            raise Refusal("file/directory path conflict")
        paths.append(path)
        if size == 0 or size > MAX_FILE_BYTES:
            raise Refusal("per-file size")
        total += size
        if total > MAX_TOTAL_BYTES:
            raise Refusal("total size")
        if kind == 1:
            exec_count += 1
            has_entry |= path == signed_entry
        if kind == 2 and path == signed_icon:
            has_icon = True
        file_data = data[payload_cursor:payload_cursor+size]
        if len(file_data) != size or hashlib.sha256(file_data).digest() != digest:
            raise Refusal("file hash/length")
        records.append((kind, path, size, digest))
        payload_cursor += size
    if total != payload_len or payload_cursor != len(data) or exec_count == 0 or not has_entry or not has_icon:
        raise Refusal("payload accounting or required file")
    signature = data[metadata_len:metadata_len+64]
    payload = data[payload_at:]
    return ParsedBundle(metadata, signature, payload, tuple(records), payload_at, hashlib.sha256(data).digest())


def default_fixtures() -> dict[str, bytes]:
    main = b"fixture-main-image"
    helper = b"helper!"
    icon = b"PNGfake"
    valid = build_bundle([
        (1, b"bin/editor", main),
        (1, b"bin/helper", helper),
        (2, b"icons/editor.bin", icon),
    ])
    traversal = build_bundle([
        (2, b"../escape", b"escape!"),
        (1, b"bin/editor", main),
        (1, b"bin/helper", helper),
        (2, b"icons/editor.bin", icon),
    ])
    duplicate = build_bundle([
        (1, b"bin/editor", main),
        (2, b"bin/editor", b"duplicate"),
        (2, b"icons/editor.bin", icon),
    ])
    prefix_conflict = build_bundle([
        (2, b"bin", b"not-a-directory"),
        (1, b"bin/editor", main),
        (2, b"icons/editor.bin", icon),
    ])
    large_payload = bytes((i * 13 + 7) & 0xff for i in range(32 * 1024))
    large_manifest = make_manifest(icon=b"", associations=())
    large = build_bundle([(1, b"bin/editor", large_payload)], manifest=large_manifest)
    reserved_manifest = make_manifest(
        entry=b"APB1.record", icon=b"", associations=()
    )
    reserved_record = build_bundle(
        [(1, b"APB1.record", b"reserved")], manifest=reserved_manifest
    )

    def resign_metadata(raw: bytes, update) -> bytes:
        metadata_len = HEADER_BYTES + MANIFEST_BYTES + int.from_bytes(raw[16:24], "little")
        metadata = bytearray(raw[:metadata_len])
        update(metadata)
        payload = raw[metadata_len + 64:]
        metadata = bytes(metadata)
        return metadata + sign(RFC_SEED, DOMAIN + metadata) + payload

    noncanonical = resign_metadata(valid, lambda metadata: metadata.__setitem__(
        HEADER_BYTES + 112, metadata[HEADER_BYTES + 112] | 0x80))
    oversized_file = resign_metadata(valid, lambda metadata: metadata.__setitem__(
        slice(HEADER_BYTES + MANIFEST_BYTES + 8, HEADER_BYTES + MANIFEST_BYTES + 16),
        (MAX_FILE_BYTES + 1).to_bytes(8, "little")))
    oversized_total = resign_metadata(valid, lambda metadata: metadata.__setitem__(
        slice(24, 32), (MAX_TOTAL_BYTES + 1).to_bytes(8, "little")))
    return {
        "editor.apb1": valid,
        "traversal.apb1": traversal,
        "duplicate.apb1": duplicate,
        "prefix-conflict.apb1": prefix_conflict,
        "large.apb1": large,
        "noncanonical.apb1": noncanonical,
        "reserved-record.apb1": reserved_record,
        "oversized-file.apb1": oversized_file,
        "oversized-total.apb1": oversized_total,
    }


def write_fixtures(directory: Path) -> None:
    directory.mkdir(parents=True, exist_ok=True)
    for name, data in default_fixtures().items():
        (directory / name).write_bytes(data)


if __name__ == "__main__":
    destination = Path(__file__).resolve().parents[1] / "userspace/arena-platform/tests/data"
    write_fixtures(destination)
    for name, data in default_fixtures().items():
        try:
            parsed = parse_bundle(data)
            detail = f"sha256={parsed.bundle_digest.hex()}"
        except Refusal as exc:
            detail = f"signed negative fixture ({exc})"
        print(f"{destination / name}: {len(data)} bytes {detail}")
