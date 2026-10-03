"""ADR-0046 host-side reference for the accepted on-disk config record.

Not the guest store: this reference makes the exact layout and recovery
policy executable before integrating a service. Only fsd-visible objects
are inputs; AFS1 commit-record corruption/rollback is out of scope.
"""

import struct
from dataclasses import dataclass

from afs1 import fnv1a64

MAGIC = b"ARCFG8V1"
SECTOR = 512
PREFIX = "cfg8-"
MAX_GENERATIONS = 8
MAX_PAYLOAD = 32


class ConfigCorrupt(ValueError):
    """Do not silently return an older generation when state is malformed."""


@dataclass(frozen=True)
class Record:
    sequence: int
    payload: bytes


@dataclass(frozen=True)
class Recovery:
    current: Record | None
    pending: str | None

    @property
    def next_name(self) -> str | None:
        if self.pending is not None:
            return self.pending
        n = 1 if self.current is None else self.current.sequence + 1
        return name(n) if n <= MAX_GENERATIONS else None


def name(sequence: int) -> str:
    if not 1 <= sequence <= MAX_GENERATIONS:
        raise ValueError("generation out of bounds")
    return f"{PREFIX}{sequence:02d}"


def pack(sequence: int, payload: bytes) -> bytes:
    name(sequence)
    if len(payload) > MAX_PAYLOAD:
        raise ValueError("config payload too long")
    body = bytearray(SECTOR - 8)
    struct.pack_into("<8sIIQH", body, 0, MAGIC, 1, 0, sequence, len(payload))
    body[32:32 + len(payload)] = payload
    return bytes(body) + struct.pack("<Q", fnv1a64(body))


def unpack(data: bytes, sequence: int) -> Record:
    if len(data) != SECTOR:
        raise ConfigCorrupt("nonempty generation has wrong size")
    if data[:8] != MAGIC or struct.unpack_from("<I", data, 8)[0] != 1:
        raise ConfigCorrupt("unknown record magic/version")
    if struct.unpack_from("<I", data, 12)[0] != 0:
        raise ConfigCorrupt("reserved header is not zero")
    if struct.unpack_from("<Q", data, 16)[0] != sequence:
        raise ConfigCorrupt("record sequence disagrees with name")
    length = struct.unpack_from("<H", data, 24)[0]
    if length > MAX_PAYLOAD or any(data[26:32]) or any(data[32 + length:504]):
        raise ConfigCorrupt("invalid length or nonzero reserved bytes")
    if fnv1a64(data[:504]) != struct.unpack_from("<Q", data, 504)[0]:
        raise ConfigCorrupt("record checksum mismatch")
    return Record(sequence, data[32:32 + length])


def recover(objects: list[tuple[str, bytes]]) -> Recovery:
    """Consume the full fsd-visible namespace, never only the latest file.

    Other applications' objects are allowed; *any* cfg8- name must be
    canonical. An empty next file is an interrupted CREATE, not a value.
    The guest will use LS to enumerate and FS_READ to retrieve each file.
    """
    generations: dict[int, bytes] = {}
    for obj_name, data in objects:
        if not obj_name.startswith(PREFIX):
            continue
        suffix = obj_name[len(PREFIX):]
        if (len(suffix) != 2 or not suffix.isascii() or not suffix.isdigit()
                or not 1 <= int(suffix) <= MAX_GENERATIONS
                or obj_name != name(int(suffix))):
            raise ConfigCorrupt("unexpected reserved config filename")
        seq = int(suffix)
        if seq in generations:
            raise ConfigCorrupt("duplicate config generation")
        generations[seq] = data
    if not generations:
        return Recovery(None, None)
    if set(generations) != set(range(1, max(generations) + 1)):
        raise ConfigCorrupt("missing generation before a newer one")
    last: Record | None = None
    pending: str | None = None
    for seq in range(1, max(generations) + 1):
        data = generations[seq]
        if not data:
            if seq != max(generations):
                raise ConfigCorrupt("empty generation before a newer one")
            pending = name(seq)
        else:
            last = unpack(data, seq)
    return Recovery(last, pending)
