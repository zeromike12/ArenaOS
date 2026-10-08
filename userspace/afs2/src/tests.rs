//! Host proofs of the Rust AFS2 engine (the format's host model is
//! `tools/afs2.py`; `tools/test_afs2_rust.py` cross-checks images both ways).
extern crate std;
use super::*;
use std::boxed::Box;
use std::collections::{BTreeMap, BTreeSet};
use std::string::{String, ToString};
use std::vec;
use std::vec::Vec;

/// In-memory device recording every write (block, bytes) in order.
pub struct Mem {
    pub img: Vec<u8>,
    pub log: Vec<(usize, Vec<u8>)>,
}
impl Device for Mem {
    fn read(&mut self, block: u64, buf: &mut [u8; BLOCK]) -> Result<()> {
        let at = block as usize * BLOCK;
        buf.copy_from_slice(self.img.get(at..at + BLOCK).ok_or(Error::Io)?);
        Ok(())
    }
    fn write(&mut self, block: u64, buf: &[u8; BLOCK]) -> Result<()> {
        let at = block as usize * BLOCK;
        self.img
            .get_mut(at..at + BLOCK)
            .ok_or(Error::Io)?
            .copy_from_slice(buf);
        self.log.push((at, buf.to_vec()));
        Ok(())
    }
    fn write_sector(&mut self, block: u64, buf: &[u8; SECTOR]) -> Result<()> {
        let at = block as usize * BLOCK;
        self.img
            .get_mut(at..at + SECTOR)
            .ok_or(Error::Io)?
            .copy_from_slice(buf);
        self.log.push((at, buf.to_vec()));
        Ok(())
    }
}

type Vol = Volume<Mem>;

fn fresh(blocks: u64) -> Box<Vol> {
    let mut v = Box::new(Vol::empty());
    let dev = Mem {
        img: vec![0; blocks as usize * BLOCK],
        log: vec![],
    };
    v.format(dev, blocks, 0x4152_454e_4132_4653, 0).unwrap();
    v
}

fn remount(img: Vec<u8>) -> Result<Box<Vol>> {
    let mut v = Box::new(Vol::empty());
    v.mount(Mem { img, log: vec![] })?;
    Ok(v)
}

fn image(v: &mut Vol) -> Vec<u8> {
    v.device().unwrap().img.clone()
}

/// Whole namespace: path -> file bytes (None = directory).
pub fn walk(v: &mut Vol) -> BTreeMap<String, Option<Vec<u8>>> {
    let mut out = BTreeMap::new();
    let root = v.root_id().unwrap();
    rec(v, root, String::new(), &mut out);
    out
}
fn rec(v: &mut Vol, dir: u64, prefix: String, out: &mut BTreeMap<String, Option<Vec<u8>>>) {
    out.insert(
        if prefix.is_empty() {
            "/".into()
        } else {
            prefix.clone()
        },
        None,
    );
    let mut after: Vec<u8> = vec![];
    loop {
        let mut batch = [Entry::EMPTY; 7];
        let n = v.list(dir, &after, &mut batch).unwrap();
        if n == 0 {
            break;
        }
        for e in &batch[..n] {
            let path = std::format!(
                "{}/{}",
                prefix,
                String::from_utf8(e.name().to_vec()).unwrap()
            );
            if e.typ == DIR {
                rec(v, e.object, path, out);
            } else {
                let size = v.stat(e.object).unwrap().size as usize;
                let mut data = vec![0u8; size];
                assert_eq!(v.read(e.object, 0, &mut data).unwrap(), size);
                out.insert(path, Some(data));
            }
        }
        after = batch[n - 1].name().to_vec();
    }
}

/// Structural audit: every referenced block is claimed exactly once and the
/// committed bitmap is exactly the referenced set; parents, generations and
/// entry counts agree.
pub fn check(v: &mut Vol) {
    let mut seen = BTreeSet::new();
    let mut claim = |b: u64| assert!(b != 0 && seen.insert(b), "block {b} claimed twice");
    claim(v.root);
    for b in &v.bitmap_blocks[..v.nbitmaps] {
        claim(*b);
    }
    v.begin();
    let mut live = 0;
    let mut p = [0u8; PAYLOAD];
    for li in 0..PTRS {
        let leaf = v.tx_root[li];
        if leaf == 0 {
            continue;
        }
        claim(leaf);
        for j in 0..RECORDS_PER_LEAF {
            let index = (li * RECORDS_PER_LEAF + j) as u32;
            let r = v.get_index(index).unwrap();
            if r.typ == 0 {
                continue;
            }
            live += 1;
            if r.depth >= 1 {
                claim(r.map);
                v.meta(r.map, KIND_MAPNODE, &mut p).unwrap();
                let top: Vec<u64> = (0..PTRS).map(|i| le64(&p, i * 8)).collect();
                for b in top {
                    if b == 0 {
                        continue;
                    }
                    claim(b);
                    if r.depth == 2 {
                        v.meta(b, KIND_MAPNODE, &mut p).unwrap();
                        for i in 0..PTRS {
                            let d = le64(&p, i * 8);
                            if d != 0 {
                                claim(d);
                            }
                        }
                    }
                }
            }
            if r.typ == DIR {
                let mut entries = 0;
                let mut prev_last: Option<Vec<u8>> = None;
                for i in 0..r.dir_blocks {
                    let b = v.map_get(&r, u64::from(i)).unwrap();
                    let mut names = vec![];
                    v.each_entry(b, |name, ci, cg, ct| {
                        names.push((name.to_vec(), ci, cg, ct));
                        true
                    })
                    .unwrap();
                    assert!(!names.is_empty(), "empty directory block kept");
                    if let Some(last) = &prev_last {
                        assert!(names[0].0 > *last, "directory blocks out of global order");
                    }
                    prev_last = Some(names.last().unwrap().0.clone());
                    for (_, ci, cg, ct) in names {
                        entries += 1;
                        let c = v.get_index(ci).unwrap();
                        assert_eq!(
                            (c.typ, c.generation, c.parent),
                            (ct, cg, oid(index, r.generation))
                        );
                    }
                }
                assert_eq!(entries, r.entries);
                assert_eq!(r.size, u64::from(r.dir_blocks) * BLOCK as u64);
            }
        }
    }
    assert_eq!(live, v.used_objects);
    for b in 0..v.total {
        let used = bit(&v.bitmap, b);
        let expected = b < FIRST_DATA || seen.contains(&b);
        assert_eq!(used, expected, "bitmap disagrees at block {b}");
    }
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn id_of(v: &mut Vol, path: &str) -> u64 {
    let mut cur = v.root_id().unwrap();
    for part in path.split('/').filter(|p| !p.is_empty()) {
        cur = v.lookup(cur, part.as_bytes()).unwrap().0;
    }
    cur
}

fn parent_name(path: &str) -> (&str, &str) {
    let i = path.rfind('/').unwrap();
    (if i == 0 { "/" } else { &path[..i] }, &path[i + 1..])
}

#[test]
fn random_operations_match_a_model_and_survive_remount() {
    let mut rng = Rng(0x00af_5200_1234_5678);
    let mut v = fresh(2048);
    let mut model: BTreeMap<String, Option<Vec<u8>>> = BTreeMap::new();
    model.insert("/".into(), None);
    let names = [
        "a",
        "b",
        "notes.txt",
        "Documents",
        "z-long-name-with-spaces and ünïcode",
        "x",
    ];
    for step in 0..700 {
        let dirs: Vec<String> = model
            .iter()
            .filter(|(_, d)| d.is_none())
            .map(|(k, _)| k.clone())
            .collect();
        let files: Vec<String> = model
            .iter()
            .filter(|(_, d)| d.is_some())
            .map(|(k, _)| k.clone())
            .collect();
        let dir = dirs[rng.below(dirs.len() as u64) as usize].clone();
        let join = |d: &str, n: &str| {
            if d == "/" {
                std::format!("/{n}")
            } else {
                std::format!("{d}/{n}")
            }
        };
        let name = names[rng.below(names.len() as u64) as usize];
        let path = join(&dir, name);
        let did = id_of(&mut v, &dir);
        match rng.below(8) {
            0 | 1 => {
                let r = v.create(did, name.as_bytes(), step);
                if model.contains_key(&path) {
                    assert_eq!(r, Err(Error::Exist));
                } else {
                    r.unwrap();
                    model.insert(path, Some(vec![]));
                }
            }
            2 => {
                let r = v.mkdir(did, name.as_bytes(), step);
                if model.contains_key(&path) {
                    assert_eq!(r, Err(Error::Exist));
                } else {
                    r.unwrap();
                    model.insert(path, None);
                }
            }
            3 | 4 if !files.is_empty() => {
                let f = files[rng.below(files.len() as u64) as usize].clone();
                let id = id_of(&mut v, &f);
                let off = rng.below(3 * BLOCK as u64 + 100);
                let len = rng.below(2 * BLOCK as u64 + 300) as usize;
                let data: Vec<u8> = (0..len).map(|i| (i as u64 ^ step) as u8).collect();
                v.write(id, off, &data, step).unwrap();
                let m = model.get_mut(&f).unwrap().as_mut().unwrap();
                if m.len() < off as usize + len {
                    m.resize(off as usize + len, 0);
                }
                m[off as usize..off as usize + len].copy_from_slice(&data);
            }
            5 if !files.is_empty() => {
                let f = files[rng.below(files.len() as u64) as usize].clone();
                let id = id_of(&mut v, &f);
                let size = rng.below(3 * BLOCK as u64);
                v.truncate(id, size, step).unwrap();
                model
                    .get_mut(&f)
                    .unwrap()
                    .as_mut()
                    .unwrap()
                    .resize(size as usize, 0);
            }
            6 => {
                // unlink / rmdir of a random entry under `dir`
                let kids: Vec<String> = model
                    .keys()
                    .filter(|k| k.as_str() != "/" && parent_name(k).0 == dir)
                    .cloned()
                    .collect();
                if kids.is_empty() {
                    continue;
                }
                let k = kids[rng.below(kids.len() as u64) as usize].clone();
                let (_, n) = parent_name(&k);
                if model[&k].is_some() {
                    v.unlink(did, n.as_bytes(), step).unwrap();
                    model.remove(&k);
                } else {
                    let empty = !model.keys().any(|x| x.starts_with(&std::format!("{k}/")));
                    let r = v.rmdir(did, n.as_bytes(), step);
                    if empty {
                        r.unwrap();
                        model.remove(&k);
                    } else {
                        assert_eq!(r, Err(Error::NotEmpty));
                    }
                }
            }
            _ => {
                // rename / move of a random entry to a random directory
                let all: Vec<String> = model
                    .keys()
                    .filter(|k| k.as_str() != "/")
                    .cloned()
                    .collect();
                if all.is_empty() {
                    continue;
                }
                let src = all[rng.below(all.len() as u64) as usize].clone();
                let (sp, sn) = parent_name(&src);
                let (sp, sn) = (sp.to_string(), sn.to_string());
                let dst = join(&dir, name);
                let spid = id_of(&mut v, &sp);
                let r = v.rename(spid, sn.as_bytes(), did, name.as_bytes(), step);
                let into_self = dir == src || dir.starts_with(&std::format!("{src}/"));
                if dst == src {
                    r.unwrap();
                } else if model.contains_key(&dst) {
                    assert_eq!(r, Err(Error::Exist));
                } else if model[&src].is_none() && into_self {
                    assert_eq!(r, Err(Error::Loop));
                } else {
                    r.unwrap();
                    let moved: Vec<(String, Option<Vec<u8>>)> = model
                        .iter()
                        .filter(|(k, _)| **k == src || k.starts_with(&std::format!("{src}/")))
                        .map(|(k, d)| (k.clone(), d.clone()))
                        .collect();
                    for (k, d) in moved {
                        model.remove(&k);
                        model.insert(std::format!("{dst}{}", &k[src.len()..]), d);
                    }
                }
            }
        }
        if step % 7 == 0 {
            let img = image(&mut v);
            v = remount(img).unwrap();
        }
        assert_eq!(walk(&mut v), model, "step {step}");
        check(&mut v);
    }
}

#[test]
fn directories_with_many_entries_split_and_merge_in_global_order() {
    let mut v = fresh(4096);
    let root = v.root_id().unwrap();
    let d = v.mkdir(root, b"many", 1).unwrap();
    let mut names: Vec<String> = (0..600)
        .map(|i| std::format!("entry-{:05}-{}", (i * 7919) % 600, "x".repeat(i % 40)))
        .collect();
    for n in &names {
        v.create(d, n.as_bytes(), 2).unwrap();
    }
    check(&mut v);
    assert!(v.stat(d).unwrap().entries == 600);
    names.sort();
    let w = walk(&mut v);
    let got: Vec<&String> = w.keys().filter(|k| k.starts_with("/many/")).collect();
    assert_eq!(got.len(), 600);
    for (i, n) in names.iter().enumerate() {
        if i % 3 != 0 {
            v.unlink(d, n.as_bytes(), 3).unwrap();
        }
    }
    check(&mut v);
    for (i, n) in names.iter().enumerate() {
        if i % 3 == 0 {
            v.unlink(d, n.as_bytes(), 3).unwrap();
        }
    }
    check(&mut v);
    assert_eq!(v.stat(d).unwrap().entries, 0);
    v.rmdir(root, b"many", 4).unwrap();
    check(&mut v);
}

/// Every prefix of every mutating operation's writes remounts to exactly
/// the old or exactly the new namespace (and a torn commit sector to old).
#[test]
fn crash_prefixes_are_exactly_old_or_new() {
    type Op = fn(&mut Vol) -> Result<()>;
    let ops: Vec<(&str, Op)> = vec![
        ("mkdir", |v| {
            let r = v.root_id()?;
            v.mkdir(r, b"d2", 5).map(|_| ())
        }),
        ("create", |v| {
            let r = v.root_id()?;
            v.create(r, b"new.txt", 5).map(|_| ())
        }),
        ("write", |v| {
            let f = id_of(v, "/d/f");
            v.write(f, 100, &[7u8; 9000], 5).map(|_| ())
        }),
        ("replace", |v| {
            let f = id_of(v, "/d/f");
            v.write(f, 0, &[9u8; 600], 5).map(|_| ())
        }),
        ("truncate", |v| {
            let f = id_of(v, "/d/f");
            v.truncate(f, 1000, 5)
        }),
        ("rename", |v| {
            let d = id_of(v, "/d");
            v.rename(d, b"f", d, b"g", 5).map(|_| ())
        }),
        ("move", |v| {
            let d = id_of(v, "/d");
            let r = v.root_id()?;
            v.rename(d, b"f", r, b"f-moved", 5).map(|_| ())
        }),
        ("move-dir", |v| {
            let r = v.root_id()?;
            let e = id_of(v, "/e");
            v.rename(r, b"d", e, b"d-in-e", 5).map(|_| ())
        }),
        ("unlink", |v| {
            let d = id_of(v, "/d");
            v.unlink(d, b"f", 5)
        }),
        ("rmdir", |v| {
            let r = v.root_id()?;
            v.rmdir(r, b"e", 5)
        }),
    ];
    let mut base = fresh(512);
    let root = base.root_id().unwrap();
    let d = base.mkdir(root, b"d", 1).unwrap();
    base.mkdir(root, b"e", 1).unwrap();
    let f = base.create(d, b"f", 1).unwrap();
    base.write(f, 0, &[3u8; 5000], 1).unwrap();
    let pre = image(&mut base);
    for (label, op, lowest) in ops
        .iter()
        .flat_map(|(l, o)| [(*l, *o, false), (*l, *o, true)])
    {
        let mut v = remount(pre.clone()).unwrap();
        v.lowest_first = lowest;
        let old = walk(&mut v);
        v.device().unwrap().log.clear();
        op(&mut v).unwrap();
        let new = walk(&mut v);
        assert_ne!(old, new, "{label} changed nothing");
        let log = v.device().unwrap().log.clone();
        // The commit sector is the last write and the only sector write.
        assert_eq!(log.last().unwrap().1.len(), SECTOR, "{label}");
        let mut prefixes = 0;
        for k in 0..=log.len() {
            let mut img = pre.clone();
            for (at, data) in &log[..k] {
                img[*at..*at + data.len()].copy_from_slice(data);
            }
            let mut m = remount(img).unwrap();
            let got = walk(&mut m);
            let expect = if k == log.len() { &new } else { &old };
            assert_eq!(&got, expect, "{label}: prefix {k}/{}", log.len());
            check(&mut m);
            prefixes += 1;
        }
        // A torn commit sector (half written) is the old state.
        let mut img = pre.clone();
        for (at, data) in &log[..log.len() - 1] {
            img[*at..*at + data.len()].copy_from_slice(data);
        }
        let (at, data) = log.last().unwrap();
        img[*at..*at + 256].copy_from_slice(&data[..256]);
        assert_eq!(
            walk(&mut remount(img).unwrap()),
            old,
            "{label}: torn commit"
        );
        std::println!("[afs2-rust] crash prefixes {label} (lowest-first {lowest}): {prefixes}");
    }
}

#[test]
fn refusals_write_nothing_and_capacity_holds() {
    let mut v = fresh(128);
    let root = v.root_id().unwrap();
    let f = v.create(root, b"big", 1).unwrap();
    let before = image(&mut v);
    v.device().unwrap().log.clear();
    // Larger than the volume: refused before any write.
    assert_eq!(v.write(f, 0, &vec![1u8; 200 * BLOCK], 2), Err(Error::NoSpc));
    assert!(v.device().unwrap().log.is_empty());
    assert_eq!(image(&mut v), before);
    for bad in [&b""[..], b".", b"..", b"a/b", b"\x01", b"\xff\xfe"] {
        assert_eq!(v.create(root, bad, 2), Err(Error::Inval));
    }
    assert_eq!(v.create(root, &[b'n'; 256], 2), Err(Error::Inval));
    v.create(root, &[b'n'; 255], 2).unwrap();
    assert_eq!(v.create(root, b"big", 2), Err(Error::Exist));
    assert_eq!(v.mkdir(f, b"x", 2), Err(Error::NotDir));
    assert_eq!(v.unlink(root, b"nope", 2), Err(Error::NoEnt));
    // Stale ids never name a new object.
    v.unlink(root, b"big", 3).unwrap();
    let again = v.create(root, b"big", 4).unwrap();
    assert_ne!(again, f);
    assert_eq!(split(again).0, split(f).0);
    assert_eq!(v.stat(f), Err(Error::NoEnt));
    check(&mut v);
}

#[test]
fn ten_thousand_objects_and_a_sixteen_mebibyte_file() {
    let mut v = fresh(16384);
    let root = v.root_id().unwrap();
    let mut dirs = vec![];
    for i in 0..20 {
        dirs.push(
            v.mkdir(root, std::format!("dir-{i}").as_bytes(), 1)
                .unwrap(),
        );
    }
    for i in 0..10_000 {
        v.create(dirs[i % 20], std::format!("file-{i:05}").as_bytes(), 2)
            .unwrap();
    }
    assert_eq!(v.statfs().objects, 10_021);
    let big = v.create(root, b"sixteen.bin", 3).unwrap();
    let chunk: Vec<u8> = (0..1 << 20).map(|i| (i % 251) as u8).collect();
    for m in 0..16u64 {
        v.write(big, m << 20, &chunk, 4).unwrap();
    }
    assert_eq!(v.stat(big).unwrap().size, 16 << 20);
    let mut back = vec![0u8; 4096];
    v.read(big, (15 << 20) + 12345, &mut back).unwrap();
    assert_eq!(back[0], ((12345) % 251) as u8);
    let img = image(&mut v);
    let mut v = remount(img).unwrap();
    check(&mut v);
    std::println!("[afs2-rust] capacity: {:?}", v.statfs());
}

#[test]
fn mount_fails_closed_on_corruption() {
    let mut v = fresh(256);
    let root = v.root_id().unwrap();
    v.create(root, b"a", 1).unwrap();
    let img = image(&mut v);
    remount(img.clone()).unwrap();
    // Flip one byte of the current object root: corruption, not fallback.
    let cur_root = v.root as usize;
    let mut bad = img.clone();
    bad[cur_root * BLOCK + 17] ^= 1;
    assert_eq!(remount(bad).err(), Some(Error::Corrupt));
    // Both commit slots invalid.
    let mut bad = img.clone();
    bad[BLOCK] ^= 1;
    bad[2 * BLOCK] ^= 1;
    assert_eq!(remount(bad).err(), Some(Error::Corrupt));
    // Superblock damaged.
    let mut bad = img;
    bad[3] ^= 1;
    assert_eq!(remount(bad).err(), Some(Error::Corrupt));
}

/// Every crash prefix of a format, from a blank region and from an
/// interrupted earlier import (a committed volume without the marker),
/// is either `never_committed` (formatted again) or mounts and audits
/// clean; the complete format is a mountable empty root. A torn first
/// commit is never committed.
#[test]
fn format_prefixes_are_never_committed_or_mountable() {
    let blocks = 512u64;
    let mut old = fresh(blocks);
    let root = old.root_id().unwrap();
    let d = old.mkdir(root, b"half-imported", 1).unwrap();
    let f = old.create(d, b"f", 1).unwrap();
    old.write(f, 0, &[5u8; 7000], 1).unwrap();
    let populated = image(&mut old);
    for (label, pre) in [("blank", vec![0u8; blocks as usize * BLOCK]), ("re-format", populated)] {
        let mut v = Box::new(Vol::empty());
        let mut dev = Mem { img: pre.clone(), log: vec![] };
        assert_eq!(never_committed(&mut dev).unwrap(), label == "blank");
        v.format(dev, blocks, 7, 3).unwrap();
        let log = v.device().unwrap().log.clone();
        let new_tree = walk(&mut v);
        let (mut formatted, mut mounted) = (0, 0);
        for k in 0..=log.len() {
            let mut img = pre.clone();
            for (at, data) in &log[..k] {
                img[*at..*at + data.len()].copy_from_slice(data);
            }
            let mut dev = Mem { img: img.clone(), log: vec![] };
            if never_committed(&mut dev).unwrap() {
                assert_ne!(k, log.len(), "{label}: a complete format is committed");
                formatted += 1;
                continue;
            }
            let mut m = remount(img).unwrap_or_else(|e| panic!("{label}: prefix {k} neither blank nor mountable: {e:?}"));
            // Before the new superblock, an earlier committed generation of
            // the interrupted volume (its marker still absent) may mount;
            // filesd formats it again. The complete format is the new root.
            let got = walk(&mut m);
            if k == log.len() {
                assert_eq!(got, new_tree, "{label}: complete format");
            } else {
                assert_eq!(label, "re-format", "blank prefix {k} mounted");
                assert!(!got.keys().any(|p| p.contains("afs1-import-complete")));
            }
            check(&mut m);
            mounted += 1;
        }
        // The first commit torn after 256 bytes: still never committed.
        let mut img = pre.clone();
        for (at, data) in &log[..log.len() - 1] {
            img[*at..*at + data.len()].copy_from_slice(data);
        }
        let (at, data) = log.last().unwrap();
        img[*at..*at + 256].copy_from_slice(&data[..256]);
        assert!(never_committed(&mut Mem { img, log: vec![] }).unwrap(), "{label}: torn first commit");
        std::println!("[afs2-rust] format crash prefixes ({label}): {} writes, {formatted} never-committed, {mounted} mountable", log.len());
    }
    // A volume past its first commit holds a valid record in both slots:
    // garbage in either one never makes it "never committed", and with
    // both destroyed it fails closed instead of being formatted.
    let mut v = fresh(blocks);
    let r = v.root_id().unwrap();
    v.mkdir(r, b"x", 1).unwrap();
    let img = image(&mut v);
    for slot in [1usize, 2] {
        let mut bad = img.clone();
        bad[slot * BLOCK..slot * BLOCK + 100].fill(0xee);
        assert!(!never_committed(&mut Mem { img: bad.clone(), log: vec![] }).unwrap());
        assert!(remount(bad).is_ok(), "slot {slot}: the other generation mounts");
    }
    let mut bad = img.clone();
    bad[BLOCK..BLOCK + 100].fill(0xee);
    bad[2 * BLOCK..2 * BLOCK + 100].fill(0xee);
    assert!(!never_committed(&mut Mem { img: bad.clone(), log: vec![] }).unwrap());
    assert!(matches!(remount(bad), Err(Error::Corrupt)));
}
