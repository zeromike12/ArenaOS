//! ArenaOS minimal shell — the first input-driven program (M4.6,
//! ADR-0020).
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
//!   channel.
//!
//! The shell is a plain program: no libc, no allocator — fixed buffers,
//! byte-wise command matching, its own decimal/hex formatters, output
//! chunked to the dispatcher's 256-byte debug_write bound. The loop:
//! prompt → `SYS_CONSOLE_READ` (blocks; the kernel echoes what the user
//! types) → match a builtin → answer. Builtins v1: `help`, `ps`,
//! `echo`, `spawn`, `shutdown`.
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

// ---- the ABI surface this image uses (frozen registry, ADR-0017…0020) ----

const SYS_DEBUG_WRITE: u64 = 1;
const SYS_THREAD_EXIT: u64 = 2;
const SYS_WAIT: u64 = 11;
const SYS_SPAWN: u64 = 12;
const SYS_CONSOLE_READ: u64 = 13;
const SYS_PROC_LIST: u64 = 14;
const SYS_SHUTDOWN: u64 = 15;

/// Dispatcher's largest accepted debug_write (kernel-side WRITE_MAX).
const WRITE_MAX: usize = 256;
/// The kernel console's line bound (console::LINE_MAX) — reading with a
/// smaller max would truncate, and the shell has no use for halves.
const LINE_LEN: usize = 120;

// ---- the grants spawn_init made, in slot order (mirrors entry.rs) ---------

const SLOT_POWER: u64 = 0;
const SLOT_IMAGE: u64 = 1; // registry image 0 = the M4.3 test payload
const SLOT_NOTIF: u64 = 2;
/// The badge this shell lends its children's exits.
const SPAWN_BADGE: u64 = 0x5AA5;

const EXIT_WRITE_REFUSED: u64 = 97;
const EXIT_CONSOLE_REFUSED: u64 = 98;
const EXIT_PANIC: u64 = 99;

// ---- fixed buffers (.bss — the loader's zero-fill is their init) ----------

static mut LINE: [u8; LINE_LEN] = [0; LINE_LEN];
static mut OUT: [u8; WRITE_MAX] = [0; WRITE_MAX];
/// (pid, threads) pairs from SYS_PROC_LIST — 32 pairs, the whole table.
static mut PROCS: [u64; 64] = [0; 64];

// ---- syscall wrappers (ABI v1: RAX = nr, args RDI/RSI/RDX/R10/R8/R9; the
// ---- kernel preserves only RBX/RBP/R12–R15, so everything else — including
// ---- the argument registers and the hardware-consumed RCX/R11 — is
// ---- declared clobbered; the stub runs on the kernel stack, `nostack`) ----

unsafe fn syscall1(nr: u64, a0: u64) -> i64 {
    let ret: i64;
    // SAFETY: caller contract — ABI v1 register arguments; the clobber
    // list is the documented caller-saved set.
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") nr as i64 => ret,
            inlateout("rdi") a0 => _,
            lateout("rsi") _,
            lateout("rdx") _,
            lateout("r8") _,
            lateout("r9") _,
            lateout("r10") _,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    ret
}

unsafe fn syscall2(nr: u64, a0: u64, a1: u64) -> i64 {
    let ret: i64;
    // SAFETY: as syscall1.
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") nr as i64 => ret,
            inlateout("rdi") a0 => _,
            inlateout("rsi") a1 => _,
            lateout("rdx") _,
            lateout("r8") _,
            lateout("r9") _,
            lateout("r10") _,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    ret
}

unsafe fn syscall5(nr: u64, a0: u64, a1: u64, a2: u64, a3: u64, a4: u64) -> i64 {
    let ret: i64;
    // SAFETY: as syscall1 (a3/a4 ride R10/R8 per ABI v1).
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") nr as i64 => ret,
            inlateout("rdi") a0 => _,
            inlateout("rsi") a1 => _,
            inlateout("rdx") a2 => _,
            inlateout("r10") a3 => _,
            inlateout("r8") a4 => _,
            lateout("r9") _,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    ret
}

// ---- console output --------------------------------------------------------

/// Write `buf` to the console, chunked to the dispatcher's bound. A
/// refusal has no recovery — the console is the shell's only voice, so
/// the honest outcome is a diagnostic exit.
fn write_all(mut buf: &[u8]) {
    while !buf.is_empty() {
        let n = if buf.len() > WRITE_MAX { WRITE_MAX } else { buf.len() };
        // SAFETY: `buf` is always shell-owned memory inside the
        // registered user regions (rodata in the text segment, or the
        // OUT/.bss statics); wrapper contract as above.
        let w = unsafe { syscall2(SYS_DEBUG_WRITE, buf.as_ptr() as u64, n as u64) };
        if w != n as i64 {
            // SAFETY: thread_exit diverges; the code is the contract.
            unsafe { syscall2(SYS_THREAD_EXIT, EXIT_WRITE_REFUSED, 0) };
        }
        buf = &buf[n..];
    }
}

fn write_str(s: &str) {
    write_all(s.as_bytes());
}

/// The composed-line writer: bytes accumulate in OUT, `flush` sends
/// them. Single-threaded image; OUT is touched only through here.
struct Out {
    n: usize,
}

impl Out {
    fn new() -> Out {
        Out { n: 0 }
    }
    fn push(&mut self, b: u8) {
        if self.n < WRITE_MAX {
            // SAFETY: OUT is this image's own .bss static, written
            // single-threaded at a bounds-checked offset.
            unsafe { *core::ptr::addr_of_mut!(OUT).cast::<u8>().add(self.n) = b };
            self.n += 1;
        }
    }
    fn str(&mut self, s: &str) {
        for b in s.bytes() {
            self.push(b);
        }
    }
    fn bytes(&mut self, s: &[u8]) {
        for &b in s {
            self.push(b);
        }
    }
    fn u64(&mut self, v: u64) {
        let mut tmp = [0u8; 20];
        let mut n = 0usize;
        let mut x = v;
        loop {
            tmp[n] = b'0' + (x % 10) as u8;
            n += 1;
            x /= 10;
            if x == 0 {
                break;
            }
        }
        while n > 0 {
            n -= 1;
            self.push(tmp[n]);
        }
    }
    fn i64(&mut self, v: i64) {
        if v < 0 {
            self.push(b'-');
            self.u64(v.wrapping_neg() as u64);
        } else {
            self.u64(v as u64);
        }
    }
    fn hex(&mut self, v: u64) {
        self.str("0x");
        for i in (0..16).rev() {
            let d = ((v >> (i * 4)) & 0xF) as u8;
            self.push(if d < 10 { b'0' + d } else { b'a' + d - 10 });
        }
    }
    /// CRLF — debug_write copies raw bytes (no \n translation), and the
    /// shell talks to a terminal.
    fn crlf(&mut self) {
        self.push(b'\r');
        self.push(b'\n');
    }
    fn flush(&mut self) {
        // SAFETY: OUT[..n] are bytes this Out wrote; single-threaded.
        let buf =
            unsafe { core::slice::from_raw_parts(core::ptr::addr_of!(OUT).cast::<u8>(), self.n) };
        write_all(buf);
        self.n = 0;
    }
}

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

const BANNER: &str = "ArenaOS shell v0.4 (M4.6, ADR-0020) — the first input-driven program.\r\n";
const PROMPT: &str = "arena> ";
const HELP: &str = "commands:\r\n  help        this text\r\n  ps          list live processes (pid + thread count)\r\n  echo TEXT   print TEXT\r\n  spawn       spawn image 0 (the M4.3 test payload), wait for its exit badge\r\n  shutdown    halt the machine (Power cap, slot 0)\r\n";

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
