#!/usr/bin/env python3
"""Independent host reference for the ADR-0083 native startup record.

This Python codec is intentionally separate from the no_std Rust parser. Its
only private bytes are a descriptive startup vector; it creates no kernel
capability or launch authority.
"""
from __future__ import annotations

import struct
from pathlib import Path

PAGE_BYTES = 4096
HEADER_BYTES = 128
ARG_MAX = 32
ENV_MAX = 32
CAP_MAX = 4
STRINGS_MAX = 3072
INSTANCE_SLOTS = 32
NONE = 0xFFFF
RIGHT_READ = 1
RIGHT_WRITE = 2
RIGHT_COPY = 4
RIGHT_DESTROY = 8
RIGHTS_MASK = 15
CAP_SHARED_REGION = 7
CAP_NOTIFICATION = 3
CAP_BOOT_IMAGE = 6
CAP_BADGED_ENDPOINT = 12

# Header offsets; these deliberately match the ADR table, not Rust symbols.
OFF_TOTAL = 8
OFF_FLAGS = 12
OFF_APP_ID = 16
OFF_INSTANCE = 48
OFF_GENERATION = 52
OFF_ARGC = 60
OFF_ENVC = 62
OFF_CAPC = 64
OFF_ARGS = 68
OFF_ENV = 72
OFF_CAPS = 76
OFF_STRINGS = 80
OFF_STRINGS_LEN = 84
OFF_CWD = 88
OFF_STDIN = 90
OFF_STDOUT = 92
OFF_STDERR = 94
OFF_PAGE_SIZE = 96
OFF_ENTRY = 104
OFF_BASE = 112
OFF_CLOCK = 120


class Refusal(ValueError):
    pass


def _u16(data: bytes | bytearray, at: int) -> int:
    return struct.unpack_from("<H", data, at)[0]


def _u32(data: bytes | bytearray, at: int) -> int:
    return struct.unpack_from("<I", data, at)[0]


def _u64(data: bytes | bytearray, at: int) -> int:
    return struct.unpack_from("<Q", data, at)[0]


def _put(data: bytearray, fmt: str, at: int, value: int) -> None:
    struct.pack_into(fmt, data, at, value)


def _valid_id(value: bytes) -> bool:
    end = value.find(b"\0")
    return (
        len(value) == 32
        and 1 <= end <= 31
        and value[end:] == bytes(32 - end)
        and (value[0:1].islower() or value[0:1].isdigit())
        and all(byte in b"abcdefghijklmnopqrstuvwxyz0123456789.-" for byte in value[:end])
    )


def encode_sample() -> bytes:
    app_id = b"com.arena.editor" + bytes(32 - len(b"com.arena.editor"))
    return _encode_record(
        app_id=app_id,
        args=(b"com.arena.editor", b"notes.txt"),
        env=(b"LANG=en",),
        caps=((1, 1, CAP_BADGED_ENDPOINT, RIGHT_WRITE), (2, 5, 1, RIGHT_READ)),
        instance=3,
        generation=77,
        flags=1,
        refs=(0, NONE, NONE, NONE),
        entry=0x0040_0120,
        base=0x0040_0000,
        clock_us=1234,
    )


def encode_runtime_sample(
    cap_rights: int = RIGHT_READ | RIGHT_WRITE,
    instance_slot: int = 4,
    include_boot_image: bool = False,
) -> bytes:
    app_id = b"com.arena.startup" + bytes(32 - len(b"com.arena.startup"))
    caps = [(1, 5, CAP_NOTIFICATION, cap_rights)]
    if include_boot_image:
        caps.append((2, 5, CAP_BOOT_IMAGE, RIGHT_READ))
    return _encode_record(
        app_id=app_id,
        args=(b"startup-probe", b"alpha"),
        env=(b"MODE=proof",),
        caps=tuple(caps),
        instance=instance_slot,
        generation=42,
        flags=0,
        refs=(NONE, NONE, NONE, NONE),
        entry=0x0020_0000,
        base=0x0020_0000,
        clock_us=987654,
    )


def _encode_record(
    *, app_id: bytes, args: tuple[bytes, ...], env: tuple[bytes, ...],
    caps: tuple[tuple[int, int, int, int], ...], instance: int,
    generation: int, flags: int, refs: tuple[int, int, int, int],
    entry: int, base: int, clock_us: int,
) -> bytes:
    if len(app_id) != 32:
        raise Refusal("application ID width")
    caps_off = HEADER_BYTES
    args_off = caps_off + len(caps) * 16
    env_off = args_off + len(args) * 8
    strings_off = env_off + len(env) * 8
    pool = b"".join(args + env)
    total = strings_off + len(pool)
    page = bytearray(PAGE_BYTES)
    page[:4] = b"ARST"
    _put(page, "<H", 4, 2)
    _put(page, "<H", 6, HEADER_BYTES)
    _put(page, "<I", OFF_TOTAL, total)
    _put(page, "<I", OFF_FLAGS, flags)
    page[OFF_APP_ID:OFF_APP_ID + 32] = app_id
    _put(page, "<H", OFF_INSTANCE, instance)
    _put(page, "<Q", OFF_GENERATION, generation)
    _put(page, "<H", OFF_ARGC, len(args))
    _put(page, "<H", OFF_ENVC, len(env))
    _put(page, "<H", OFF_CAPC, len(caps))
    _put(page, "<I", OFF_ARGS, args_off)
    _put(page, "<I", OFF_ENV, env_off)
    _put(page, "<I", OFF_CAPS, caps_off)
    _put(page, "<I", OFF_STRINGS, strings_off)
    _put(page, "<I", OFF_STRINGS_LEN, len(pool))
    for at, reference in zip((OFF_CWD, OFF_STDIN, OFF_STDOUT, OFF_STDERR), refs):
        _put(page, "<H", at, reference)
    _put(page, "<I", OFF_PAGE_SIZE, PAGE_BYTES)
    _put(page, "<Q", OFF_ENTRY, entry)
    _put(page, "<Q", OFF_BASE, base)
    _put(page, "<Q", OFF_CLOCK, clock_us)

    for i, (slot, role, kind, rights) in enumerate(caps):
        at = caps_off + i * 16
        struct.pack_into("<HBBI", page, at, slot, role, kind, rights)
    cursor = 0
    for table, strings in ((args_off, args), (env_off, env)):
        for i, value in enumerate(strings):
            struct.pack_into("<II", page, table + i * 8, cursor, len(value))
            cursor += len(value)
    page[strings_off:strings_off + len(pool)] = pool
    parse(bytes(page))
    return bytes(page)


def parse(page: bytes) -> dict[str, object]:
    if len(page) != PAGE_BYTES:
        raise Refusal("page length")
    if page[:4] != b"ARST":
        raise Refusal("magic")
    if _u16(page, 4) != 2 or _u16(page, 6) != HEADER_BYTES:
        raise Refusal("version/header")
    if any(page[a:b] != bytes(b-a) for a, b in ((50, 52), (66, 68), (100, 104))):
        raise Refusal("reserved header")
    total = _u32(page, OFF_TOTAL)
    argc, envc, capc = _u16(page, OFF_ARGC), _u16(page, OFF_ENVC), _u16(page, OFF_CAPC)
    strings_len = _u32(page, OFF_STRINGS_LEN)
    instance, generation = _u16(page, OFF_INSTANCE), _u64(page, OFF_GENERATION)
    flags = _u32(page, OFF_FLAGS)
    if not HEADER_BYTES <= total <= PAGE_BYTES or not 1 <= argc <= ARG_MAX or not 0 <= envc <= ENV_MAX:
        raise Refusal("bounds")
    if capc > CAP_MAX or strings_len > STRINGS_MAX or instance >= INSTANCE_SLOTS or generation == 0:
        raise Refusal("bounds")
    if flags & ~7:
        raise Refusal("flags")
    app_id = page[OFF_APP_ID:OFF_APP_ID + 32]
    if not _valid_id(app_id):
        raise Refusal("application ID")
    caps_off, args_off = _u32(page, OFF_CAPS), _u32(page, OFF_ARGS)
    env_off, strings_off = _u32(page, OFF_ENV), _u32(page, OFF_STRINGS)
    expected_args = HEADER_BYTES + capc * 16
    expected_env = expected_args + argc * 8
    expected_strings = expected_env + envc * 8
    if (caps_off, args_off, env_off, strings_off) != (HEADER_BYTES, expected_args, expected_env, expected_strings):
        raise Refusal("noncanonical offsets")
    if strings_off + strings_len != total or any(page[total:]):
        raise Refusal("noncanonical total/tail")
    if _u32(page, OFF_PAGE_SIZE) != PAGE_BYTES:
        raise Refusal("page size")
    entry, base = _u64(page, OFF_ENTRY), _u64(page, OFF_BASE)
    if not entry or not base or base % PAGE_BYTES or not base <= entry < 0x0000_8000_0000_0000:
        raise Refusal("entry/base")

    descriptors = []
    roles = {}
    for i in range(capc):
        at = caps_off + i * 16
        slot, role, kind, rights = struct.unpack_from("<HBBI", page, at)
        if slot != i + 1 or role not in (1, 2, 3, 4, 5) or not 1 <= kind <= 13:
            raise Refusal("capability descriptor")
        if not rights or rights & ~RIGHTS_MASK or any(page[at + 8:at + 16]):
            raise Refusal("capability rights/reserved")
        if role != 5 and role in roles:
            raise Refusal("duplicate capability role")
        roles[role] = i
        if role == 1 and (kind != CAP_BADGED_ENDPOINT or not rights & RIGHT_WRITE):
            raise Refusal("CWD capability")
        descriptors.append((slot, role, kind, rights))

    refs = tuple(_u16(page, at) for at in (OFF_CWD, OFF_STDIN, OFF_STDOUT, OFF_STDERR))
    wanted_roles = (1, 2, 3, 4)
    for ref, role in zip(refs, wanted_roles):
        expected = roles.get(role, NONE)
        if ref != expected or (ref != NONE and ref >= capc):
            raise Refusal("role reference")

    values = []
    cursor = 0
    for table, count, environment in ((args_off, argc, False), (env_off, envc, True)):
        group = []
        for i in range(count):
            offset, length = struct.unpack_from("<II", page, table + i * 8)
            if offset != cursor or offset + length > strings_len:
                raise Refusal("string bounds/order")
            value = page[strings_off + offset:strings_off + offset + length]
            if b"\0" in value or (not length and (environment or (not environment and i == 0))):
                raise Refusal("string encoding")
            cursor += length
            group.append(value)
        values.append(tuple(group))
    if cursor != strings_len:
        raise Refusal("string tail")
    return {
        "application_id": app_id,
        "instance_slot": instance,
        "instance_generation": generation,
        "flags": flags,
        "arguments": values[0],
        "environment": values[1],
        "capabilities": tuple(descriptors),
        "references": refs,
        "page_size": PAGE_BYTES,
        "entry": entry,
        "load_base": base,
        "clock_us": _u64(page, OFF_CLOCK),
        "total_bytes": total,
    }


def validate_startup_cap(observed: tuple[int, int, int], pages: int) -> bool:
    return observed[0] == CAP_SHARED_REGION and observed[2] == (RIGHT_READ | RIGHT_DESTROY) and pages == 1


def write_fixture(path: Path, data: bytes | None = None) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(encode_sample() if data is None else data)


def write_runtime_fixtures(directory: Path) -> tuple[Path, Path, Path]:
    valid = directory / "startup-runtime-v2.bin"
    mismatch = directory / "startup-runtime-bad-rights-v2.bin"
    heap_slot = directory / "startup-runtime-heap-slot-v2.bin"
    write_fixture(valid, encode_runtime_sample())
    write_fixture(mismatch, encode_runtime_sample(RIGHT_READ | RIGHT_DESTROY))
    write_fixture(
        heap_slot,
        encode_runtime_sample(
            RIGHT_READ | RIGHT_WRITE | RIGHT_COPY | RIGHT_DESTROY,
            instance_slot=5,
            include_boot_image=True,
        ),
    )
    return valid, mismatch, heap_slot


if __name__ == "__main__":
    root = Path(__file__).resolve().parents[1]
    fixture = root / "userspace/arena-platform/tests/data/startup-v2.bin"
    write_fixture(fixture)
    print(f"wrote {fixture} ({fixture.stat().st_size} bytes)")
    for runtime_fixture in write_runtime_fixtures(root / "userspace/arena-runtime/tests/data"):
        print(f"wrote {runtime_fixture} ({runtime_fixture.stat().st_size} bytes)")
