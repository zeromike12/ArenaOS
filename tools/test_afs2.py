#!/usr/bin/env python3
"""AFS2 host model proofs (ADR-0076): property, crash-prefix and RED controls.

1. Randomized operation sequences (nested mkdir, create, write/overwrite,
   growth past one map node, truncate, unlink, rmdir, rename and
   cross-directory move including directory moves and refused cycles,
   invalid names) are applied both to AFS2 and to an independent
   dictionary model; after EVERY operation the volume is remounted from
   its bytes, structurally audited (`afs2.check`) and its whole namespace
   compared with the model.
2. Crash prefixes: for every mutating operation kind, every prefix of the
   operation's block writes is applied to the pre-operation image; each
   must remount to exactly the old namespace (all but the last write) or
   exactly the new one (all writes). The commit sector is the last write.
3. Refusals before mutation: no-space and object capacity, invalid names,
   existing targets, non-empty rmdir — the image stays byte-identical.
4. Fail closed: a corrupted metadata block referenced by the newest commit
   refuses to mount instead of falling back to an older generation.
5. RED controls (`--red`): in-place metadata write, early free, rename
   without the parent update, and a rename published in two commits each
   make the proofs above fail; restored source passes.
"""
import random
import subprocess
import sys
from pathlib import Path
import afs2
from afs2 import FsError

ROOT = Path(__file__).resolve().parent.parent


class Model:
    """Independent namespace model: path -> bytes (file) | None (dir)."""

    def __init__(self):
        self.fs = {"/": None}

    def snapshot(self):
        return dict(self.fs)

    def children(self, d):
        pre = "/" if d == "/" else d + "/"
        return [p for p in self.fs if p != d and p.startswith(pre) and "/" not in p[len(pre):]]


def join(d, name):
    return ("" if d == "/" else d) + "/" + name


def ops_round(seed, steps, total_blocks=4096):
    rng = random.Random(seed)
    vol = afs2.Volume(afs2.mkfs(total_blocks))
    m = Model()
    names = ["a", "b", "notes.txt", "Ünïcode", "x" * 200, "dir", "photo.raw", "z9"]
    counts = {}

    def dirs():
        return [p for p, v in m.fs.items() if v is None]

    def files():
        return [p for p, v in m.fs.items() if v is not None]

    for step in range(steps):
        op = rng.choice(["mkdir", "create", "write", "write", "truncate", "unlink", "rmdir",
                         "rename", "rename", "bad"])
        before = vol.image()
        expect_error = None
        try:
            if op == "mkdir":
                d = rng.choice(dirs())
                name = rng.choice(names)
                path = join(d, name)
                expect_error = "EEXIST" if path in m.fs else None
                vol.mkdir(vol.resolve(d), name.encode())
                m.fs[path] = None
            elif op == "create":
                d = rng.choice(dirs())
                name = rng.choice(names)
                path = join(d, name)
                expect_error = "EEXIST" if path in m.fs else None
                vol.create(vol.resolve(d), name.encode())
                m.fs[path] = b""
            elif op == "write" and files():
                f = rng.choice(files())
                old = m.fs[f]
                off = rng.choice([0, len(old), rng.randrange(0, len(old) + 1), len(old) + 5000,
                                  rng.randrange(0, 3 * afs2.BLOCK)])
                n = rng.choice([1, 100, afs2.BLOCK, 3 * afs2.BLOCK + 17, 600 * afs2.BLOCK // 1000])
                data = bytes(rng.randrange(256) for _ in range(n))
                vol.write(vol.resolve(f), off, data)
                grown = old + bytes(max(0, off - len(old)))
                m.fs[f] = grown[:off] + data + grown[off + n:]
            elif op == "truncate" and files():
                f = rng.choice(files())
                size = rng.choice([0, 1, len(m.fs[f]) // 2, len(m.fs[f]) + 3000])
                vol.truncate(vol.resolve(f), size)
                m.fs[f] = (m.fs[f] + bytes(size))[:size]
            elif op == "unlink" and files():
                f = rng.choice(files())
                d, name = f.rsplit("/", 1)
                vol.unlink(vol.resolve(d or "/"), name.encode())
                del m.fs[f]
            elif op == "rmdir":
                cand = [p for p in dirs() if p != "/"]
                if cand:
                    d = rng.choice(cand)
                    expect_error = "ENOTEMPTY" if m.children(d) else None
                    parent, name = d.rsplit("/", 1)
                    vol.rmdir(vol.resolve(parent or "/"), name.encode())
                    del m.fs[d]
            elif op == "rename":
                src = rng.choice([p for p in m.fs if p != "/"] or ["/"])
                if src != "/":
                    dst_dir = rng.choice(dirs())
                    new = rng.choice(names)
                    target = join(dst_dir, new)
                    sp, sname = src.rsplit("/", 1)
                    cycle = m.fs[src] is None and (dst_dir == src or dst_dir.startswith(src + "/"))
                    # Precedence (ADR-0076): an existing target, then a cycle.
                    if target in m.fs and target != src:
                        expect_error = "EEXIST"
                    elif cycle:
                        expect_error = "ELOOP"
                    vol.rename(vol.resolve(sp or "/"), sname.encode(), vol.resolve(dst_dir),
                               new.encode())
                    if target != src:
                        moved = {}
                        for p in list(m.fs):
                            if p == src or p.startswith(src + "/"):
                                moved[target + p[len(src):]] = m.fs.pop(p)
                        m.fs.update(moved)
            elif op == "bad":
                d = rng.choice(dirs())
                expect_error = "EINVAL"
                vol.create(vol.resolve(d), rng.choice([b"", b".", b"..", b"a/b", b"x\x00", b"\xff",
                                                       b"y" * 256, b"tab\t"]))
            if expect_error:
                raise AssertionError(f"step {step} {op}: expected {expect_error}, succeeded")
        except FsError as e:
            if e.code != expect_error:
                raise AssertionError(f"step {step} {op}: unexpected {e.code} (wanted {expect_error})")
            assert vol.image() == before, f"step {step}: refused {op} changed the image"
        counts[op] = counts.get(op, 0) + 1
        vol = afs2.Volume(vol.image())  # remount from bytes after every operation
        afs2.check(vol)
        got = afs2.walk(vol)
        if got != m.snapshot():
            diff = set(got.items()) ^ set(m.snapshot().items())
            raise AssertionError(f"step {step} {op}: namespace differs {sorted(diff)[:4]}")
    return counts, vol


def crash_prefixes(total_blocks=2048):
    """Every write prefix of every mutating op remounts to old or new."""
    vol = afs2.Volume(afs2.mkfs(total_blocks))
    r = vol.root_id()
    docs = vol.mkdir(r, b"Documents")
    sub = vol.mkdir(docs, b"Projects")
    f = vol.create(docs, b"draft.txt")
    vol.write(f, 0, b"hello world" * 500)
    big = vol.create(r, b"big.bin")
    vol.write(big, 0, bytes(range(256)) * 16 * 520)  # crosses one map node
    cases = [
        ("mkdir", lambda v: v.mkdir(v.resolve("/Documents"), b"New Folder")),
        ("create", lambda v: v.create(v.resolve("/Documents/Projects"), b"plan.md")),
        ("write", lambda v: v.write(v.resolve("/Documents/draft.txt"), 4000, b"X" * 9000)),
        ("replace", lambda v: (v.truncate(v.resolve("/Documents/draft.txt"), 0),)),
        ("truncate", lambda v: v.truncate(v.resolve("/big.bin"), 5000)),
        ("rename", lambda v: v.rename(v.resolve("/Documents"), b"draft.txt",
                                      v.resolve("/Documents"), b"final.txt")),
        ("move", lambda v: v.rename(v.resolve("/Documents"), b"draft.txt",
                                    v.resolve("/Documents/Projects"), b"draft.txt")),
        ("move-dir", lambda v: v.rename(v.resolve("/Documents"), b"Projects", v.resolve("/"),
                                        b"Projects")),
        ("unlink", lambda v: v.unlink(v.resolve("/"), b"big.bin")),
        ("rmdir", lambda v: v.rmdir(v.resolve("/Documents"), b"Projects")),
    ]
    proven = {}
    for name, op in cases:
        base = vol.image()
        old = afs2.walk(afs2.Volume(base))
        v = afs2.Volume(base)
        op(v)
        writes = list(v.last_writes)
        new = afs2.walk(afs2.Volume(v.image()))
        assert old != new, name
        assert writes[-1][0] in (afs2.BLOCK, 2 * afs2.BLOCK) and len(writes[-1][1]) == afs2.SECTOR
        for k in range(len(writes) + 1):
            img = bytearray(base)
            for off, data in writes[:k]:
                img[off:off + len(data)] = data
            got = afs2.Volume(bytes(img))
            afs2.check(got)
            state = afs2.walk(got)
            want = new if k == len(writes) else old
            assert state == want, f"{name}: prefix {k}/{len(writes)} is a hybrid state"
        proven[name] = len(writes)
    return proven


def refusals():
    vol = afs2.Volume(afs2.mkfs(96))  # tiny: exhaust space quickly
    r = vol.root_id()
    d = vol.mkdir(r, b"d")
    vol.create(d, b"x")
    f = vol.create(r, b"fill")
    data = b"\xaa" * afs2.BLOCK
    n = 0
    while True:
        before = vol.image()
        try:
            vol.write(f, n * afs2.BLOCK, data)
            n += 1
        except FsError as e:
            assert e.code == "ENOSPC", e.code
            assert vol.image() == before, "ENOSPC write mutated the image"
            break
    afs2.check(afs2.Volume(vol.image()))
    before = vol.image()
    for call in (lambda: vol.rmdir(r, b"d"), lambda: vol.create(r, b"fill"),
                 lambda: vol.create(r, b"bad/name"), lambda: vol.unlink(r, b"d"),
                 lambda: vol.rename(r, b"d", d, b"inside"),
                 lambda: vol.mkdir(r, b"more")):
        try:
            call()
            raise AssertionError("refusal accepted")
        except FsError:
            assert vol.image() == before
    return n


def capacity(target=10_000):
    """≥ 10,000 objects and a ≥ 16 MiB file on a 64 MiB volume."""
    vol = afs2.Volume(afs2.mkfs(16384))
    r = vol.root_id()
    big = vol.create(r, b"sixteen.bin")
    chunk = bytes(range(256)) * 1024  # 256 KiB
    for i in range(64):
        vol.write(big, i * len(chunk), chunk)
    assert vol.stat(big)["size"] == 16 * 1024 * 1024
    assert vol.read(big, 15 * 1024 * 1024, 300) == chunk[:300]
    d = vol.mkdir(r, b"many")
    for i in range(target):
        vol.create(d, f"item-{i:05d}.txt".encode())
    vol2 = afs2.Volume(vol.image())
    afs2.check(vol2)
    assert vol2.stat(vol2.resolve("/many"))["entries"] == target
    names = []
    after = b""
    while True:
        batch = vol2.list(vol2.resolve("/many"), after, 500)
        if not batch:
            break
        names += [n for n, _, _ in batch]
        after = batch[-1][0]
    assert names == sorted(names) and len(names) == target
    long = "ü" * 127  # 254 UTF-8 bytes
    vol2.create(r, long.encode())
    assert vol2.lookup(r, long.encode())
    return vol2.statfs()


def fail_closed():
    vol = afs2.Volume(afs2.mkfs(512))
    d = vol.mkdir(vol.root_id(), b"keep")
    vol.create(d, b"file")
    img = bytearray(vol.image())
    leaf = vol._root_ptrs()[0]
    img[leaf * afs2.BLOCK + 10] ^= 0x40
    try:
        afs2.Volume(bytes(img)).stat(d)
        raise AssertionError("corrupt leaf accepted")
    except FsError as e:
        assert e.code == "CORRUPT"
    img = bytearray(vol.image())
    img[vol.root * afs2.BLOCK + 100] ^= 1
    try:
        afs2.Volume(bytes(img))
        raise AssertionError("corrupt newest root silently fell back")
    except FsError as e:
        assert e.code == "CORRUPT"
    return True


def run():
    total = {}
    for seed in (1, 2, 3, 4):
        counts, _ = ops_round(seed, 260)
        for k, v in counts.items():
            total[k] = total.get(k, 0) + v
    proven = crash_prefixes()
    n = refusals()
    fs = capacity()
    fail_closed()
    print(f"[afs2] {sum(total.values())} randomized operations remounted/audited/matched {total}")
    print(f"[afs2] crash prefixes exact old-or-new: {proven}")
    print(f"[afs2] no-space after {n} blocks, refusals byte-identical; capacity {fs}; fail-closed PASS")


RED = [
    # In-place metadata: a changed object leaf overwrites its old block.
    (b"            nb = self.alloc()\n            writes.append((nb, (\"leaf\", bytes(payload))))",
     b"            nb = old or self.alloc()\n            writes.append((nb, (\"leaf\", bytes(payload))))",
     "in-place-metadata"),
    # Early free: a block the committed generation references is reusable
    # inside the same transaction.
    (b"            self.pending_free.add(block)",
     b"            self.pending_free.add(block)\n            Volume._clear_bit(self.work, block)",
     "early-free"),
    # Rename forgets the moved object's parent update.
    (b"        child.parent = dst_dir\n", b"", "missing-parent-update"),
    # Half-published rename: the removal commits before the insertion.
    (b"            self.store_dir(src_dir, src, sblocks)\n            src.mtime = wall_us\n"
     b"            src.version += 1\n            self.put(src_dir, src)\n            dst = self.get(dst_dir)",
     b"            self.store_dir(src_dir, src, sblocks)\n            src.mtime = wall_us\n"
     b"            src.version += 1\n            self.put(src_dir, src)\n            self.commit(wall_us)\n"
     b"            self.__init__(self.vol)\n            dst = self.get(dst_dir)",
     "half-published-rename"),
]


def red():
    src = ROOT / "tools/afs2.py"
    original = src.read_bytes()
    try:
        for before, after, label in RED:
            assert original.count(before) == 1, label
            src.write_bytes(original.replace(before, after, 1))
            r = subprocess.run([sys.executable, __file__], cwd=ROOT / "tools",
                               capture_output=True, text=True, timeout=3600)
            assert r.returncode != 0 and "SyntaxError" not in r.stderr, \
                f"{label}: proofs did not fail\n{r.stdout[-500:]}{r.stderr[-1500:]}"
            print(f"[afs2-red] {label}: RED PASS ({r.stderr.strip().splitlines()[-1][:120]})")
    finally:
        src.write_bytes(original)
    assert src.read_bytes() == original
    r = subprocess.run([sys.executable, __file__], cwd=ROOT / "tools", capture_output=True, text=True)
    assert r.returncode == 0, r.stderr[-2000:]
    print(f"[afs2-red] {len(RED)} RED controls; byte-exact restore; GREEN")


if __name__ == "__main__":
    if "--red" in sys.argv:
        red()
    else:
        run()
