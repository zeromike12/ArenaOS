//! ArenaOS minimal shell — the first input-driven program (M4.6,
//! ADR-0020; filesystem builtins M5.3, ADR-0023).
//!
//! A genuine cargo/rust-lld ELF64 artifact (`x86_64-unknown-none`,
//! `shell.ld`) at its own fixed window: `0x400000` text (R+X),
//! `0x410000` data+bss (R+W, NOLOAD bss). It is embedded into the
//! kernel as spawn-registry image 1 and spawned at boot as the initial
//! service through `spawn::spawn_init` — the M4.5 creation sequence
//! with kernel-literal grants:
//!
//! - slot 0: `Power` (WRITE) — the authority behind `shutdown`,
//! - slot 1: `Image{0}` (READ) — the M4.3 test payload, for `spawn`,
//! - slot 2: `Notification` (READ|WRITE) — its children's exit-badge
//!   channel,
//! - slot 3: `Endpoint` (WRITE) — the call side of fsd's filesystem
//!   service (M5.3): `ls`, `cat`, and `write` run through it.
//!
//! The shell is a plain program: no libc, no allocator — fixed buffers,
//! byte-wise command matching, the shared userspace ABI surface
//! (`userspace/abi.rs`, one wire contract for every ring-3 image),
//! output chunked to the dispatcher's 256-byte debug_write bound. The
//! loop: prompt → `SYS_CONSOLE_READ` (blocks; the kernel echoes what
//! the user types) → match a builtin → answer. Builtins: `help`, `ps`,
//! `echo`, `ls`, `cat NAME`, `write NAME TEXT`, `spawn`, `shutdown`.
//!
//! The file builtins allocate ONE 4 KiB frame lazily on first use
//! (lend-keep pattern, ADR-0022: the owned cap is consumed by the
//! self-map, the LENT copy travels with the FS calls) and reuse it for
//! the rest of the shell's life — a resident process must not leak a
//! frame per command. `write` CREATES a new file and refuses to
//! overwrite an existing one (v1 has no truncate; honest refusal over
//! silent clobber). `cat` streams the file in ≤ 3584-byte chunks.
//!
//! Exit codes (diagnostic contract with the harness): 97 the console
//! refused an output write, 98 `SYS_CONSOLE_READ` returned a typed
//! refusal (impossible for the sole reader — a kernel contract breach),
//! 99 the panic handler ran. The shell otherwise NEVER exits: the
//! machine stops through `shutdown` (Power-gated SYS_SHUTDOWN), not
//! through the shell's death.
#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[path = "../../abi.rs"]
mod abi;
use abi::*;

// ---- shell-local constants (the grants mirror entry.rs) ----------------------

/// The kernel console's line bound (console::LINE_MAX) — reading with a
/// smaller max would truncate, and the shell has no use for halves.
const LINE_LEN: usize = 120;

const SLOT_POWER: u64 = 0;
const SLOT_IMAGE: u64 = 1; // registry image 0 = the M4.3 test payload
const SLOT_NOTIF: u64 = 2;
const SLOT_FSD: u64 = 3; // M5.3: the filesystem service call side
/// The badge this shell lends its children's exits.
const SPAWN_BADGE: u64 = 0x5AA5;

/// File-I/O window slots: the owned cap (consumed by the self-map) and
/// the LENT copy that travels with the FS calls.
const SLOT_FILE_BUF: u64 = 8;
const SLOT_FILE_LENT: u64 = 9;

const EXIT_CONSOLE_REFUSED: u64 = 98;

// ---- fixed buffers (.bss — the loader's zero-fill is their init) ----------

static mut LINE: [u8; LINE_LEN] = [0; LINE_LEN];
/// (pid, threads) pairs from SYS_PROC_LIST — 32 pairs, the whole table.
static mut PROCS: [u64; 64] = [0; 64];
/// The lazily allocated file window: its VA (0 = not yet allocated).
static mut FILE_VA: u64 = 0;

// ---- byte-wise line helpers (no std, no alloc, no surprises) ---------------

fn eq(line: &[u8], cmd: &[u8]) -> bool {
    if line.len() != cmd.len() {
        return false;
    }
    let mut i = 0;
    while i < line.len() {
        if line[i] != cmd[i] {
            return false;
        }
        i += 1;
    }
    true
}

fn strip_prefix<'a>(line: &'a [u8], p: &[u8]) -> Option<&'a [u8]> {
    if line.len() < p.len() {
        return None;
    }
    let mut i = 0;
    while i < p.len() {
        if line[i] != p[i] {
            return None;
        }
        i += 1;
    }
    Some(&line[p.len()..])
}

/// Split `rest` at the first space: (word, remainder).
fn split_word(rest: &[u8]) -> (&[u8], &[u8]) {
    let mut i = 0;
    while i < rest.len() && rest[i] != b' ' {
        i += 1;
    }
    (&rest[..i], &rest[i..])
}

// ---- the FS plumbing (M5.3) ---------------------------------------------------

/// The file window: allocated/copied/mapped on first use, reused
/// forever after. Returns its VA, or 0 when the kernel refused (the
/// caller reports and moves on — a refused frame is not fatal).
fn file_va(o: &mut Out) -> u64 {
    // SAFETY: FILE_VA is this image's own .bss; the wrappers are the
    // ABI v1 contract; slots mirror the constants above.
    unsafe {
        let va = *core::ptr::addr_of!(FILE_VA);
        if va != 0 {
            return va;
        }
        let phys = syscall1(SYS_ALLOC_FRAME, SLOT_FILE_BUF);
        if phys <= 0 {
            o.str("  frame allocation refused: ");
            o.i64(phys);
            o.crlf();
            return 0;
        }
        if syscall3(SYS_CAP_COPY, SLOT_FILE_BUF, SLOT_FILE_LENT, RIGHTS_ALL) < 0 {
            o.str("  frame cap copy refused\r\n");
            return 0;
        }
        let win = syscall2(SYS_MAP_MEMORY, SLOT_FILE_BUF, 1);
        if win <= 0 {
            o.str("  frame self-map refused\r\n");
            return 0;
        }
        *core::ptr::addr_of_mut!(FILE_VA) = win as u64;
        win as u64
    }
}

/// One synchronous FS request through the granted endpoint (slot 3).
/// `msg` is the IPC v1.1 inline buffer (names IN, dirents OUT).
/// Returns (transport status, reply w0, reply w1).
fn fs_call(op: u64, w1: u64, cap: u64, msg: &mut [u8; MSG_BYTES]) -> (i64, u64, u64) {
    let mut reply = [0u64; 3];
    // SAFETY: wrapper contract; `reply`/`msg` are this thread's own
    // (registered) memory; the endpoint cap is the granted slot 3.
    let r = unsafe {
        syscall6(
            SYS_IPC_CALL,
            SLOT_FSD,
            op,
            w1,
            cap,
            reply.as_mut_ptr() as u64,
            msg.as_mut_ptr() as u64,
        )
    };
    (r, reply[0], reply[1])
}

/// Report a failed FS request honestly (transport vs. service status).
fn fs_error(o: &mut Out, what: &str, r: i64, st: u64) {
    o.str("  ");
    o.str(what);
    if r < 0 {
        o.str(": call refused (");
        o.i64(r);
        o.str(")\r\n");
    } else {
        o.str(": fs status ");
        o.i64(st as i64);
        o.str("\r\n");
    }
}

/// Put a NUL-padded name (or a length word) into the inline message.
fn msg_zero(msg: &mut [u8; MSG_BYTES]) {
    for b in msg.iter_mut() {
        *b = 0;
    }
}

// ---- the builtins ----------------------------------------------------------

fn do_ps(o: &mut Out) {
    // SAFETY: PROCS is this image's own .bss (32 (pid, threads) pairs);
    // wrapper contract.
    let r = unsafe { syscall2(SYS_PROC_LIST, core::ptr::addr_of!(PROCS) as u64, 32) };
    if r < 0 {
        o.str("  proc_list refused: ");
        o.i64(r);
        o.crlf();
        return;
    }
    // SAFETY: the kernel wrote `r` validated pairs into PROCS.
    unsafe {
        for i in 0..r as usize {
            let pid = *core::ptr::addr_of!(PROCS[2 * i]);
            let threads = *core::ptr::addr_of!(PROCS[2 * i + 1]);
            o.str("  pid ");
            o.u64(pid);
            o.str("  threads ");
            o.u64(threads);
            o.crlf();
        }
    }
}

/// `ls` — walk the FS endpoint's LS cursor to the end, one dirent per
/// inline message.
fn do_ls(o: &mut Out) {
    let mut msg = [0u8; MSG_BYTES];
    let mut cursor: u64 = 0;
    let mut files = 0u64;
    // Bounded walk: the object table holds 32 records; a cursor that
    // never ends is a service bug — report it, never spin.
    let mut steps = 0u32;
    while steps < 33 {
        steps += 1;
        msg_zero(&mut msg);
        let (r, st, _) = fs_call(FS_OP_LS, cursor, CAP_NONE, &mut msg);
        if r < 0 || st != FS_OK {
            fs_error(o, "ls", r, st);
            return;
        }
        // SAFETY: msg is this thread's own buffer; the dirent layout
        // is the FS protocol's (userspace/abi.rs).
        let (next, size, nlen) = unsafe {
            (
                core::ptr::read_unaligned(msg.as_ptr() as *const u32),
                core::ptr::read_unaligned(msg.as_ptr().add(4) as *const u64),
                core::ptr::read_unaligned(msg.as_ptr().add(12) as *const u32),
            )
        };
        if next == FS_CURSOR_END {
            o.str("  ");
            o.u64(files);
            o.str(" file(s)\r\n");
            return;
        }
        o.str("  ");
        o.bytes(&msg[16..16 + nlen as usize]);
        o.str("  ");
        o.u64(size);
        o.str(" bytes\r\n");
        files += 1;
        cursor = u64::from(next);
    }
    o.str("  ls: the cursor never ended (service bug)\r\n");
}

/// `cat NAME` — open, stream the file through the lent frame in
/// ≤ FS_XFER_MAX chunks, close. The bytes travel disk → device → the
/// shell's own frame (fsd forwards the cap; zero copy end to end).
fn do_cat(o: &mut Out, name: &[u8]) {
    if name.is_empty() || name.len() >= FS_NAME_MAX {
        o.str("  usage: cat NAME (1..31 bytes)\r\n");
        return;
    }
    let va = file_va(o);
    if va == 0 {
        return;
    }
    let mut msg = [0u8; MSG_BYTES];
    msg_zero(&mut msg);
    msg[..name.len()].copy_from_slice(name);
    let (r, st, fh) = fs_call(FS_OP_OPEN, 0, CAP_NONE, &mut msg);
    if r < 0 || st != FS_OK {
        fs_error(o, "cat: open", r, st);
        return;
    }
    let mut off: u64 = 0;
    let mut total: u64 = 0;
    loop {
        msg_zero(&mut msg);
        // SAFETY: own buffer; the length word is the protocol's.
        unsafe { core::ptr::write_unaligned(msg.as_mut_ptr() as *mut u64, FS_XFER_MAX) };
        let (r, st, n) = fs_call(FS_OP_READ, fs_rw_w1(fh, off), SLOT_FILE_LENT, &mut msg);
        if r < 0 || st != FS_OK {
            fs_error(o, "cat: read", r, st);
            break;
        }
        if n == 0 {
            break; // EOF
        }
        // SAFETY: the device DMA'd `n` bytes into the shell's own
        // mapped frame; n <= FS_XFER_MAX < 4096.
        let chunk: &[u8] = unsafe { core::slice::from_raw_parts(va as *const u8, n as usize) };
        write_all(chunk);
        total += n;
        off += n;
    }
    o.crlf();
    let (r, st, _) = fs_call(FS_OP_CLOSE, fh, CAP_NONE, &mut msg);
    if r < 0 || st != FS_OK {
        fs_error(o, "cat: close", r, st);
        return;
    }
    o.str("  (");
    o.u64(total);
    o.str(" bytes)\r\n");
}

/// `write NAME TEXT` — CREATE a new file with TEXT as its contents
/// (v1 refuses to overwrite: no truncate yet, and a silent clobber is
/// worse than an honest refusal). The text bytes ride the shell's
/// lent frame; the device DMAs them straight to the disk.
fn do_write(o: &mut Out, rest: &[u8]) {
    let (name, tail) = split_word(rest);
    let text = if tail.first() == Some(&b' ') {
        &tail[1..]
    } else {
        tail
    };
    if name.is_empty() || name.len() >= FS_NAME_MAX {
        o.str("  usage: write NAME TEXT (name 1..31 bytes)\r\n");
        return;
    }
    if text.is_empty() {
        o.str("  write: refusing to create an empty file\r\n");
        return;
    }
    let va = file_va(o);
    if va == 0 {
        return;
    }
    let mut msg = [0u8; MSG_BYTES];
    msg_zero(&mut msg);
    msg[..name.len()].copy_from_slice(name);
    let (r, st, fh) = fs_call(FS_OP_CREATE, 0, CAP_NONE, &mut msg);
    if r < 0 || st != FS_OK {
        if st == FS_ERR_EXISTS {
            o.str("  write: '");
            o.bytes(name);
            o.str("' already exists — v1 has no truncate; refusing to overwrite\r\n");
        } else {
            fs_error(o, "write: create", r, st);
        }
        return;
    }
    // SAFETY: the shell's own mapped frame; text.len() <= LINE_LEN.
    unsafe { core::ptr::copy_nonoverlapping(text.as_ptr(), va as *mut u8, text.len()) };
    msg_zero(&mut msg);
    // SAFETY: own buffer.
    unsafe { core::ptr::write_unaligned(msg.as_mut_ptr() as *mut u64, text.len() as u64) };
    let (r, st, n) = fs_call(FS_OP_WRITE, fs_rw_w1(fh, 0), SLOT_FILE_LENT, &mut msg);
    if r < 0 || st != FS_OK || n != text.len() as u64 {
        fs_error(o, "write", r, st);
        let _ = fs_call(FS_OP_CLOSE, fh, CAP_NONE, &mut msg);
        return;
    }
    let (r, st, _) = fs_call(FS_OP_CLOSE, fh, CAP_NONE, &mut msg);
    if r < 0 || st != FS_OK {
        fs_error(o, "write: close", r, st);
        return;
    }
    o.str("  wrote ");
    o.u64(n);
    o.str(" bytes to '");
    o.bytes(name);
    o.str("'\r\n");
}

/// `rm NAME` — UNLINK (M5.4): one transaction removes the name and
/// queues every sector of the file's extent chain for reclamation two
/// generations later (ADR-0023). An open file is refused honestly —
/// v1 has no unlink-at-last-close.
fn do_rm(o: &mut Out, name: &[u8]) {
    if name.is_empty() || name.len() >= FS_NAME_MAX {
        o.str("  usage: rm NAME (1..31 bytes)\r\n");
        return;
    }
    let mut msg = [0u8; MSG_BYTES];
    msg_zero(&mut msg);
    msg[..name.len()].copy_from_slice(name);
    let (r, st, _) = fs_call(FS_OP_UNLINK, 0, CAP_NONE, &mut msg);
    if r < 0 || st != FS_OK {
        if st == FS_ERR_NOT_FOUND {
            o.str("  rm: no such file: '");
            o.bytes(name);
            o.str("'\r\n");
        } else if st == FS_ERR_BUSY {
            o.str("  rm: '");
            o.bytes(name);
            o.str("' is open — v1 deletes only closed files\r\n");
        } else {
            fs_error(o, "rm", r, st);
        }
        return;
    }
    o.str("  removed '");
    o.bytes(name);
    o.str("'\r\n");
}

fn do_spawn() {
    let mut o = Out::new();
    // Empty inheritance spec (null pointer, count 0 — the child needs
    // nothing), and the shell's own notification + badge lent for the
    // child's exit (ADR-0019).
    // SAFETY: wrapper contract; slot numbers mirror entry.rs's grants.
    let r = unsafe { syscall5(SYS_SPAWN, SLOT_IMAGE, 0, 0, SLOT_NOTIF, SPAWN_BADGE) };
    if r <= 0 {
        o.str("  spawn refused: ");
        o.i64(r);
        o.crlf();
        o.flush();
        return;
    }
    o.str("  spawned pid ");
    o.u64(r as u64);
    o.str(" — waiting for its exit badge\r\n");
    o.flush();
    // The child (the untouched M4.3 payload) writes its message to the
    // console while we are parked here — the visible proof of the
    // restart story on real iron.
    // SAFETY: wrapper contract.
    let b = unsafe { syscall1(SYS_WAIT, SLOT_NOTIF) };
    let mut o = Out::new();
    if b == SPAWN_BADGE as i64 {
        o.str("  child exited, badge ");
        o.hex(b as u64);
        o.crlf();
    } else {
        o.str("  wait returned ");
        o.i64(b);
        o.str(" (expected badge ");
        o.hex(SPAWN_BADGE);
        o.str(")\r\n");
    }
    o.flush();
}

// ---- texts -----------------------------------------------------------------

const BANNER: &str = "ArenaOS shell v0.10 (M4.6 + M5.3/5.4 + M6.5, ADR-0020/0023/0028) — serial, keyboard, or console port.\r\n";
const PROMPT: &str = "arena> ";
/// One debug_write chunk (<= WRITE_MAX = 256): the help text hits the
/// wire atomically — and Out::push DROPS bytes past WRITE_MAX, so an
/// over-long HELP would silently lose its tail. Budget: 251 bytes.
const HELP: &str = "commands:\r\n  help - this text\r\n  ps - live processes\r\n  echo TEXT - print TEXT\r\n  ls - list the AFS1 files\r\n  cat NAME - print a file\r\n  write NAME TXT - create a file\r\n  rm NAME - delete a file\r\n  spawn - run image 0\r\n  shutdown - halt the machine\r\n";

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    // This image has no panicking path by construction (checked
    // indexing-free arithmetic, no allocation). If a future shell
    // reaches here, say so through the ABI itself.
    write_str("ARENAOS-SHELL: PANIC\r\n");
    // SAFETY: thread_exit diverges.
    unsafe { syscall1(SYS_THREAD_EXIT, EXIT_PANIC) };
    loop {
        core::hint::spin_loop();
    }
}

// ---- the program -----------------------------------------------------------

/// The shell's event loop. Entered by the spawn protocol's first thread
/// at ring 3 with RSP = the derived stack top; nothing here returns —
/// the machine stops through `shutdown`, and the only other exits are
/// the diagnostic codes above.
///
/// # Safety
/// As every image's `_start`: ring 3, derived stack top, kernel-loaded
/// address space, the documented grants (slots 0..3). The body is
/// ABI v1 wrappers over this image's own statics, single-threaded,
/// every static touched through addr_of/addr_of_mut only.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _start() -> ! {
    // SAFETY: this is the whole program — ABI v1 wrappers over the
    // shell's own statics, inside the address space the kernel loaded,
    // validated, and handed to ring 3. Single-threaded, no aliases:
    // every static is touched through addr_of/addr_of_mut only.
    unsafe {
        write_str(BANNER);
        loop {
            write_str(PROMPT);
            let n = syscall2(
                SYS_CONSOLE_READ,
                core::ptr::addr_of!(LINE) as u64,
                LINE_LEN as u64,
            );
            if n < 0 {
                // Typed refusal for the sole reader is a kernel contract
                // breach — report it and die diagnostically, never spin.
                let mut o = Out::new();
                o.str("console_read refused: ");
                o.i64(n);
                o.crlf();
                o.flush();
                syscall2(SYS_THREAD_EXIT, EXIT_CONSOLE_REFUSED, 0);
            }
            if n == 0 {
                continue; // blank line: straight back to the prompt
            }
            let line =
                core::slice::from_raw_parts(core::ptr::addr_of!(LINE).cast::<u8>(), n as usize);

            let mut o = Out::new();
            if eq(line, b"help") {
                o.str(HELP);
            } else if eq(line, b"ps") {
                do_ps(&mut o);
            } else if eq(line, b"echo") {
                o.crlf();
            } else if let Some(rest) = strip_prefix(line, b"echo ") {
                o.bytes(rest);
                o.crlf();
            } else if eq(line, b"ls") {
                do_ls(&mut o);
            } else if let Some(rest) = strip_prefix(line, b"cat ") {
                do_cat(&mut o, rest);
            } else if let Some(rest) = strip_prefix(line, b"write ") {
                do_write(&mut o, rest);
            } else if let Some(rest) = strip_prefix(line, b"rm ") {
                do_rm(&mut o, rest);
            } else if eq(line, b"spawn") {
                o.flush();
                do_spawn(); // does its own output (the child talks mid-flight)
                continue;
            } else if eq(line, b"shutdown") {
                o.str("shutting down...\r\n");
                o.flush();
                let r = syscall1(SYS_SHUTDOWN, SLOT_POWER);
                // Only reachable if the kernel refused the Power cap —
                // report honestly and keep serving.
                let mut o = Out::new();
                o.str("shutdown refused: ");
                o.i64(r);
                o.crlf();
            } else {
                o.str("unknown command: '");
                o.bytes(line);
                o.str("' — try 'help'\r\n");
            }
            o.flush();
        }
    }
}
