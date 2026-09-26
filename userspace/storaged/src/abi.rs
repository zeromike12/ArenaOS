//! The shared ABI surface of the storaged crate (M5.2, ADR-0022) —
//! included by BOTH binaries (`arena-storaged`, `blktest`) through
//! `#[path]`, so the driver and its test client speak the exact same
//! frozen syscall registry and block protocol. No libc, no allocator:
//! fixed buffers, volatile MMIO/ring accessors, and the shell's
//! chunked-console discipline.
//!
//! ABI v1 (ADR-0017): RAX = call number, args RDI/RSI/RDX/R10/R8/R9;
//! the kernel preserves only RBX/RBP/R12–R15, so every stub declares
//! the documented caller-saved set clobbered. Returns: negative = typed
//! status, positive = payload, 0 = OK where a call has no payload.

#![allow(dead_code)] // the two binaries use overlapping subsets

// ---- the frozen syscall registry (numbers mirror kernel syscall.rs) --------

pub const SYS_DEBUG_WRITE: u64 = 1;
pub const SYS_THREAD_EXIT: u64 = 2;
pub const SYS_IPC_CALL: u64 = 7;
pub const SYS_IPC_RECV: u64 = 8;
pub const SYS_IPC_REPLY: u64 = 9;
pub const SYS_WAIT: u64 = 11;
pub const SYS_ALLOC_FRAME: u64 = 16;
pub const SYS_MAP_MEMORY: u64 = 17;
pub const SYS_IRQ_RELAY: u64 = 18;
pub const SYS_CAP_PHYS: u64 = 19;
pub const SYS_CAP_DESTROY: u64 = 20;
pub const SYS_CAP_COPY: u64 = 21;
pub const SYS_DEV_INFO: u64 = 22;

// ---- cap/IPC constants (mirror kernel cap.rs / ipc.rs) ----------------------

pub const CAP_NONE: u64 = u64::MAX;
pub const RIGHTS_READ: u64 = 1 << 0;
pub const RIGHTS_WRITE: u64 = 1 << 1;
pub const RIGHTS_COPY: u64 = 1 << 2;
pub const RIGHTS_DESTROY: u64 = 1 << 3;
pub const RIGHTS_ALL: u64 = RIGHTS_READ | RIGHTS_WRITE | RIGHTS_COPY | RIGHTS_DESTROY;

// ---- the block protocol (ADR-0022) ------------------------------------------
//
// Request words:  w0 = sector (512-byte units), w1 = op.
// Reply words:    w0 = virtio status byte (0 = OK), w1 = device-written
//                 byte count from the used ring (informational).
// The caller's buffer travels as a LENT Untyped cap attached to the
// call — the driver learns its phys through SYS_CAP_PHYS and the device
// DMAs the caller's own frame (zero-copy). Exactly one sector per
// request in v1; the buffer cap must name at least SECTOR_BYTES.

pub const OP_READ: u64 = 0;
pub const OP_WRITE: u64 = 1;
/// The poison request: reply, then exit cleanly (a driver process with
/// parked threads must never be destroyed out from under them).
pub const OP_SHUTDOWN: u64 = 2;
pub const SECTOR_BYTES: usize = 512;
/// virtio-blk status values (virtio 1.0 §5.2.6.1).
pub const VIRTIO_BLK_S_OK: u64 = 0;
pub const VIRTIO_BLK_S_IOERR: u64 = 1;
pub const VIRTIO_BLK_S_UNSUPP: u64 = 2;

// ---- diagnostic exit codes shared by both binaries ---------------------------

pub const EXIT_OK: u64 = 42;
pub const EXIT_WRITE_REFUSED: u64 = 97;
pub const EXIT_PANIC: u64 = 99;

// ---- syscall stubs (ABI v1; see the module header) ---------------------------

pub unsafe fn syscall1(nr: u64, a0: u64) -> i64 {
    let ret: i64;
    // SAFETY: caller contract — ABI v1 register arguments; the clobber
    // list is the documented caller-saved set; `nostack` (the stub runs
    // on the kernel stack).
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

pub unsafe fn syscall2(nr: u64, a0: u64, a1: u64) -> i64 {
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

pub unsafe fn syscall3(nr: u64, a0: u64, a1: u64, a2: u64) -> i64 {
    let ret: i64;
    // SAFETY: as syscall1.
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") nr as i64 => ret,
            inlateout("rdi") a0 => _,
            inlateout("rsi") a1 => _,
            inlateout("rdx") a2 => _,
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

pub unsafe fn syscall4(nr: u64, a0: u64, a1: u64, a2: u64, a3: u64) -> i64 {
    let ret: i64;
    // SAFETY: as syscall1 (a3 rides R10 per ABI v1).
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") nr as i64 => ret,
            inlateout("rdi") a0 => _,
            inlateout("rsi") a1 => _,
            inlateout("rdx") a2 => _,
            inlateout("r10") a3 => _,
            lateout("r8") _,
            lateout("r9") _,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    ret
}

pub unsafe fn syscall5(nr: u64, a0: u64, a1: u64, a2: u64, a3: u64, a4: u64) -> i64 {
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

// ---- console output (the shell's discipline: chunked, honest exits) ----------

/// Dispatcher's largest accepted debug_write (kernel-side WRITE_MAX).
pub const WRITE_MAX: usize = 256;

static mut OUT: [u8; WRITE_MAX] = [0; WRITE_MAX];

/// Write `buf` to the console, chunked to the dispatcher's bound. A
/// refusal has no recovery — the console is this image's only voice, so
/// the honest outcome is a diagnostic exit.
pub fn write_all(mut buf: &[u8]) {
    while !buf.is_empty() {
        let n = if buf.len() > WRITE_MAX {
            WRITE_MAX
        } else {
            buf.len()
        };
        // SAFETY: `buf` is always image-owned memory inside the
        // registered user regions (rodata in the text segment, or the
        // OUT/.bss static); wrapper contract as above.
        let w = unsafe { syscall2(SYS_DEBUG_WRITE, buf.as_ptr() as u64, n as u64) };
        if w != n as i64 {
            // SAFETY: thread_exit diverges; the code is the contract.
            unsafe { syscall2(SYS_THREAD_EXIT, EXIT_WRITE_REFUSED, 0) };
        }
        buf = &buf[n..];
    }
}

pub fn write_str(s: &str) {
    write_all(s.as_bytes());
}

/// The composed-line writer: bytes accumulate in OUT, `flush` sends
/// them. Both binaries are single-threaded; OUT is touched only here.
pub struct Out {
    pub n: usize,
}

impl Out {
    pub fn new() -> Out {
        Out { n: 0 }
    }
    pub fn push(&mut self, b: u8) {
        if self.n < WRITE_MAX {
            // SAFETY: OUT is this image's own .bss static, written
            // single-threaded at a bounds-checked offset.
            unsafe { *core::ptr::addr_of_mut!(OUT).cast::<u8>().add(self.n) = b };
            self.n += 1;
        }
    }
    pub fn str(&mut self, s: &str) {
        for b in s.bytes() {
            self.push(b);
        }
    }
    pub fn u64(&mut self, v: u64) {
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
    pub fn i64(&mut self, v: i64) {
        if v < 0 {
            self.push(b'-');
            self.u64(v.wrapping_neg() as u64);
        } else {
            self.u64(v as u64);
        }
    }
    pub fn hex(&mut self, v: u64) {
        self.str("0x");
        for i in (0..16).rev() {
            let d = ((v >> (i * 4)) & 0xF) as u8;
            self.push(if d < 10 { b'0' + d } else { b'a' + d - 10 });
        }
    }
    /// CRLF — debug_write copies raw bytes (no \n translation), and the
    /// console is a terminal.
    pub fn crlf(&mut self) {
        self.push(b'\r');
        self.push(b'\n');
    }
    pub fn flush(&mut self) {
        // SAFETY: OUT[..n] are bytes this Out wrote; single-threaded.
        let buf =
            unsafe { core::slice::from_raw_parts(core::ptr::addr_of!(OUT).cast::<u8>(), self.n) };
        write_all(buf);
        self.n = 0;
    }
}

/// Log one composed line immediately (the driver's per-event voice).
pub fn log_line(f: impl FnOnce(&mut Out)) {
    let mut o = Out::new();
    f(&mut o);
    o.crlf();
    o.flush();
}

// ---- volatile memory accessors (MMIO registers + virtqueue rings) ------------
//
// Every device/ring touch goes through these: the compiler must never
// elide, merge, or invent accesses to memory a device reads and writes
// concurrently. Ordering to the DEVICE is x86-TSO's job plus the
// explicit fences the driver places around ring-index updates.

pub unsafe fn r8(va: u64) -> u8 {
    // SAFETY: caller contract — `va` is a mapped, suitably aligned
    // register/ring byte this image owns or was granted.
    unsafe { core::ptr::read_volatile(va as *const u8) }
}
pub unsafe fn w8(va: u64, v: u8) {
    // SAFETY: as r8.
    unsafe { core::ptr::write_volatile(va as *mut u8, v) }
}
pub unsafe fn r16(va: u64) -> u16 {
    // SAFETY: as r8; 2-aligned.
    unsafe { core::ptr::read_volatile(va as *const u16) }
}
pub unsafe fn w16(va: u64, v: u16) {
    // SAFETY: as r16.
    unsafe { core::ptr::write_volatile(va as *mut u16, v) }
}
pub unsafe fn r32(va: u64) -> u32 {
    // SAFETY: as r8; 4-aligned.
    unsafe { core::ptr::read_volatile(va as *const u32) }
}
pub unsafe fn w32(va: u64, v: u32) {
    // SAFETY: as r32.
    unsafe { core::ptr::write_volatile(va as *mut u32, v) }
}

/// A full-store fence: ring writes (descriptors, avail index) must be
/// globally visible before the doorbell write reaches the device.
pub fn store_fence() {
    core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
    // SAFETY: SFENCE is unprivileged and side-effect-free beyond
    // ordering this CPU's stores.
    unsafe { core::arch::asm!("sfence", options(nostack, preserves_flags)) };
}
