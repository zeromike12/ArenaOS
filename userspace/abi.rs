//! The shared userspace ABI surface (M5.2/M5.3, ADR-0022/0023) — one
//! file, included by EVERY service crate binary (`arena-storaged`,
//! `blktest`, `fsd`, `fstest`) through `#[path = "../../abi.rs"]`, so
//! both sides of every wire protocol speak from the same frozen
//! definitions: syscall registry, IPC v1.1 inline-message helpers,
//! block protocol v1.1, and the filesystem-service protocol. No libc,
//! no allocator: fixed buffers, volatile MMIO/ring accessors, and the
//! shell's chunked-console discipline.
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
pub const SYS_SPAWN: u64 = 12;
pub const SYS_CONSOLE_READ: u64 = 13;
pub const SYS_PROC_LIST: u64 = 14;
pub const SYS_SHUTDOWN: u64 = 15;
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

// ---- IPC v1.1 inline messages (M5.3, ADR-0023) -------------------------------
//
// CALL/RECV/REPLY take an OPTIONAL trailing pointer to a 64-byte
// buffer: the kernel snapshots it from the CALLER/REPLIER (owner
// context), rides it in the call slot, and hands it to the other side.
// A null pointer means "no inline message" (v1.0 behavior). Payloads
// are opaque bytes — the FS protocol below defines its layouts.

pub const MSG_BYTES: usize = 64;

// ---- the block protocol (ADR-0022, extended v1.1 by ADR-0023) ----------------
//
// Request words:  w0 = sector (512-byte units),
//                 w1 = op | (buf_offset << 8) — buf_offset selects the
//                 sector's landing spot INSIDE the caller's lent 4 KiB
//                 frame, so one frame can stage up to 8 sectors and
//                 forwarded caps keep DMA'ing the client's own memory.
//                 The driver MUST refuse offsets with
//                 buf_offset + 512 > 4096 (a wild offset could aim the
//                 device at an adjacent frame).
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
/// Every lent buffer frame is one 4 KiB page (Untyped cap granularity).
pub const BLOCK_FRAME_BYTES: u64 = 4096;

/// Pack a block-protocol request word 1 (op + in-frame buffer offset).
pub fn block_req_w1(op: u64, buf_offset: u64) -> u64 {
    op | (buf_offset << 8)
}

// ---- the filesystem-service protocol (ADR-0023) --------------------------------
//
// fsd owns AFS1 (extent data + CoW/transactional metadata) and serves
// it on its own endpoint. Client data buffers travel the same way as
// block requests: a LENT Untyped cap attached to the call, which fsd
// FORWARDS to storaged — the device DMAs straight between the disk and
// the CLIENT's frame (zero-copy end to end; fsd never maps it — lent
// caps cannot be mapped by design).
//
// Request word 0 is the op; word 1 depends on the op:
//   CREATE / OPEN  w1 ignored; msg64 = name bytes (<= FS_NAME_MAX)
//   READ / WRITE   w1 = fh | (file_offset << 8); msg64 [0..8] = length
//                  (bytes, <= 3584); send cap = client's LENT frame.
//                  v1 WRITE must start at or below EOF (gap/sparse
//                  writes answer FS_ERR_RANGE — the extent mapping is
//                  cumulative and only appends stay honest)
//   CLOSE          w1 = fh
//   UNLINK         w1 ignored; msg64 = name bytes. Transactional
//                  delete (M5.4): the object and its extent chain die
//                  with two-generation delay; an OPEN file answers
//                  FS_ERR_BUSY (v1 has no unlink-at-last-close)
//   LS             w1 = cursor (0 to start); msg64 OUT = dirent:
//                  [0..4] next cursor (FS_CURSOR_END = done),
//                  [4..12] size, [12..16] name length, [16..48] name
//   SHUTDOWN       reply, then fsd exits (same discipline as storaged)
// Reply word 0 is FS_OK or a negative FS_ERR_* status; word 1:
//   CREATE/OPEN -> file handle, READ/WRITE -> byte count,
//   SHUTDOWN -> fsd's lifetime disk-operation count, else 0.

pub const FS_OP_CREATE: u64 = 1;
pub const FS_OP_OPEN: u64 = 2;
pub const FS_OP_READ: u64 = 3;
pub const FS_OP_WRITE: u64 = 4;
pub const FS_OP_CLOSE: u64 = 5;
pub const FS_OP_LS: u64 = 6;
pub const FS_OP_SHUTDOWN: u64 = 7;
pub const FS_OP_UNLINK: u64 = 8;

pub const FS_OK: u64 = 0;
pub const FS_ERR_NOT_FOUND: u64 = (-1i64) as u64;
pub const FS_ERR_EXISTS: u64 = (-2i64) as u64;
pub const FS_ERR_BAD_FH: u64 = (-3i64) as u64;
pub const FS_ERR_TABLE_FULL: u64 = (-4i64) as u64;
pub const FS_ERR_IO: u64 = (-5i64) as u64;
pub const FS_ERR_NO_SPACE: u64 = (-6i64) as u64;
pub const FS_ERR_BAD_NAME: u64 = (-7i64) as u64;
pub const FS_ERR_CORRUPT: u64 = (-8i64) as u64;
pub const FS_ERR_RANGE: u64 = (-9i64) as u64;
pub const FS_ERR_BAD_OP: u64 = (-10i64) as u64;
pub const FS_ERR_BUSY: u64 = (-11i64) as u64;

pub const FS_NAME_MAX: usize = 32;

/// The suite's scratch-disk geometry in 512-byte sectors — mirrors
/// `tools/arena_env.py` (`SCRATCH_MIB = 8`). blktest's raw-block cycle
/// claims the LAST sector: the host-side mkfs (ADR-0023) puts AFS1's
/// metadata in sectors 0..10 and the suite's fsd allocates upward from
/// 11, so a raw write there cannot touch filesystem state.
pub const SCRATCH_TOTAL_SECTORS: u64 = 16384;

/// The deterministic test pattern shared by blktest and fstest — and
/// mirrored byte-for-byte by `tools/test_m5.py`'s on-disk verification
/// (the host parses the real committed sectors after boot).
pub fn pattern_byte(i: usize) -> u8 {
    (i as u8).wrapping_mul(31).wrapping_add(0x5A) ^ ((i >> 3) as u8)
}
pub const FS_CURSOR_END: u32 = 0xFFFF_FFFF;
/// Largest single READ/WRITE transfer: one 4 KiB frame minus the
/// largest in-frame offset the protocol can express (512 * 1 = one
/// sector alignment slack kept for the final partial sector).
pub const FS_XFER_MAX: u64 = 3584;

/// Pack the FS READ/WRITE request word 1 (handle + file offset).
pub fn fs_rw_w1(fh: u64, file_offset: u64) -> u64 {
    fh | (file_offset << 8)
}
/// virtio-blk status values (virtio 1.0 §5.2.6.1).
pub const VIRTIO_BLK_S_OK: u64 = 0;
pub const VIRTIO_BLK_S_IOERR: u64 = 1;
pub const VIRTIO_BLK_S_UNSUPP: u64 = 2;

// ---- diagnostic exit codes shared by both binaries ---------------------------

pub const EXIT_OK: u64 = 42;
/// fstest's SECOND verified-success code (M5.4): the volume already
/// held the test file at mount — it was committed by an EARLIER boot
/// and survived. The persisted branch verifies it byte-for-byte with
/// no write at all, so its device-operation contract differs from the
/// fresh-volume one; the code tells the m5 suite which contract held.
pub const EXIT_OK_PERSISTED: u64 = 43;
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

pub unsafe fn syscall6(nr: u64, a0: u64, a1: u64, a2: u64, a3: u64, a4: u64, a5: u64) -> i64 {
    let ret: i64;
    // SAFETY: as syscall1 (a5 rides R9 per ABI v1). IPC v1.1 calls
    // MUST use this stub (or explicitly zero R9): the kernel validates
    // a stale r9 as an inline-message pointer.
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") nr as i64 => ret,
            inlateout("rdi") a0 => _,
            inlateout("rsi") a1 => _,
            inlateout("rdx") a2 => _,
            inlateout("r10") a3 => _,
            inlateout("r8") a4 => _,
            inlateout("r9") a5 => _,
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
    pub fn bytes(&mut self, s: &[u8]) {
        for &b in s {
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
