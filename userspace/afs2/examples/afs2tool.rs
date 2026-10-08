//! Host cross-check tool for the Rust AFS2 engine.
//!
//! `afs2tool build IMG` formats IMG (16384 blocks) and runs a fixed
//! workload (nested directories, a split directory, a two-level map file,
//! renames, unlinks, truncation); `afs2tool dump IMG` mounts IMG and prints
//! the namespace as `path<TAB>D` or `path<TAB>F<TAB>size<TAB>fnv64` lines.
use arena_afs2::*;
use std::fs;

struct FileDev(Vec<u8>);
impl Device for FileDev {
    fn read(&mut self, block: u64, buf: &mut [u8; BLOCK]) -> Result<()> {
        let at = block as usize * BLOCK;
        buf.copy_from_slice(self.0.get(at..at + BLOCK).ok_or(Error::Io)?);
        Ok(())
    }
    fn write(&mut self, block: u64, buf: &[u8; BLOCK]) -> Result<()> {
        let at = block as usize * BLOCK;
        self.0
            .get_mut(at..at + BLOCK)
            .ok_or(Error::Io)?
            .copy_from_slice(buf);
        Ok(())
    }
    fn write_sector(&mut self, block: u64, buf: &[u8; SECTOR]) -> Result<()> {
        let at = block as usize * BLOCK;
        self.0
            .get_mut(at..at + SECTOR)
            .ok_or(Error::Io)?
            .copy_from_slice(buf);
        Ok(())
    }
}

fn dump(v: &mut Volume<FileDev>, dir: u64, prefix: &str, out: &mut Vec<String>) {
    out.push(format!(
        "{}\tD",
        if prefix.is_empty() { "/" } else { prefix }
    ));
    let mut after = Vec::new();
    loop {
        let mut batch = [Entry::EMPTY; 16];
        let n = v.list(dir, &after, &mut batch).expect("list");
        if n == 0 {
            break;
        }
        for e in &batch[..n] {
            let path = format!("{}/{}", prefix, String::from_utf8_lossy(e.name()));
            if e.typ == DIR {
                dump(v, e.object, &path, out);
            } else {
                let size = v.stat(e.object).expect("stat").size as usize;
                let mut data = vec![0u8; size];
                v.read(e.object, 0, &mut data).expect("read");
                out.push(format!("{path}\tF\t{size}\t{:016x}", fnv(&data)));
            }
        }
        after = batch[n - 1].name().to_vec();
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut v = Box::new(Volume::<FileDev>::empty());
    match args[1].as_str() {
        "build" => {
            v.format(
                FileDev(vec![0; 16384 * BLOCK]),
                16384,
                0x5255_5354,
                1_700_000_000_000_000,
            )
            .expect("format");
            let r = v.root_id().unwrap();
            let sys = v.mkdir(r, b"System", 1).unwrap();
            let users = v.mkdir(r, b"Users", 1).unwrap();
            let user = v.mkdir(users, b"user", 1).unwrap();
            let docs = v.mkdir(user, "Documents".as_bytes(), 1).unwrap();
            v.mkdir(user, b"Desktop", 1).unwrap();
            v.mkdir(user, b".Trash", 1).unwrap();
            for i in 0..400u32 {
                let f = v
                    .create(
                        docs,
                        format!("note-{i:04} with a longer name").as_bytes(),
                        2,
                    )
                    .unwrap();
                v.write(
                    f,
                    0,
                    format!("note {i}\n")
                        .repeat((i % 7) as usize + 1)
                        .as_bytes(),
                    2,
                )
                .unwrap();
            }
            for i in (0..400u32).step_by(3) {
                v.unlink(
                    docs,
                    format!("note-{i:04} with a longer name").as_bytes(),
                    3,
                )
                .unwrap();
            }
            let big = v.create(sys, b"big.bin", 4).unwrap();
            let chunk: Vec<u8> = (0..(1u32 << 20)).map(|i| (i % 253) as u8).collect();
            for m in 0..3u64 {
                v.write(big, m << 20, &chunk, 4).unwrap();
            }
            v.write(big, 600 * 4096 + 77, b"hole-crossing write", 4)
                .unwrap();
            v.truncate(big, (2 << 20) + 4095, 5).unwrap();
            v.rename(docs, b"note-0001 with a longer name", user, b"moved.txt", 6)
                .unwrap();
            v.rename(user, b"Desktop", sys, b"Desktop-moved", 6)
                .unwrap();
            v.rename(sys, b"Desktop-moved", user, b"Desktop", 7)
                .unwrap();
            fs::write(&args[2], &v.device().unwrap().0).unwrap();
        }
        "dump" => {
            v.mount(FileDev(fs::read(&args[2]).unwrap()))
                .expect("mount");
            let root = v.root_id().unwrap();
            let mut out = Vec::new();
            dump(&mut v, root, "", &mut out);
            for l in out {
                println!("{l}");
            }
            let s = v.statfs();
            println!("#statfs\t{}\t{}\t{}", s.blocks, s.free, s.objects);
        }
        _ => panic!("usage"),
    }
}
