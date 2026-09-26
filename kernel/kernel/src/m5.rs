//! Milestone 5 test suite, step 5.1 — the driver substrate (ADR-0021).
//! Runs in `kmain` after the M4 RESULT line; every test is a real machine
//! effect, in the suite discipline of M3/M4 (self-contained, no imports
//! from the other suites, ring-3 claims proven by payloads, kernel claims
//! proven by counted allocator/IPC/relay state):
//!
//! 1. `pci_scan` — the kernel's boot-time bus-0 walk (drivers::pci) found
//!    the harness's virtio-blk fixture: capability structures resolved to
//!    memory-BAR locations, MSI-X recorded, and MEM|BUS MASTER verifiably
//!    set — the kernel-policy write ring 3 never performs.
//! 2. `untyped_alloc` — a ring-3 payload allocates two OWNED frames
//!    (SYS_ALLOC_FRAME), self-maps the first (SYS_MAP_MEMORY: kernel-chosen
//!    VA, cap CONSUMED — a re-map of the emptied slot returns -2 in ring 3),
//!    writes and reads a magic word through the window; the kernel then
//!    proves the ownership promise: destroying the second cap returns
//!    exactly one frame, and proc::destroy reclaims the mapped frame —
//!    total teardown frame-exact.
//! 3. `mmio_user` — the kernel mints an Mmio cap over the HPET main-counter
//!    page; a ring-3 payload self-maps it read-only and observes the counter
//!    strictly increase across a bounded delay; the cap survives mapping
//!    (descriptive, not owned), and teardown frees exactly the RAM — the
//!    MMIO leaf is skipped, never returned to the frame allocator.
//! 4. `irq_relay` — the interrupt→notification bridge: registration seams
//!    (range, badge, double-register, release), then the LIVE path: a parked
//!    kernel waiter, a LAPIC self-IPI at relay vector 48, the stub's dual
//!    EOI, `relay::handle`, `ipc::notify`, and the waiter waking with the
//!    exact badge; a spurious delivery on an unregistered vector is counted
//!    and survived.
//!
//! Markers: `m5:test:<name>`, `m5: RESULT`.

use crate::arch::x86_64::{paging, syscall};
use crate::cap::{self, Cap, CapObj};
use crate::drivers::{intc, pci};
use crate::frames;
use crate::ipc;
use crate::log::{log_error as error, log_info as info, write_marker};
use crate::proc;
use crate::relay;
use crate::sched;
use crate::sync::SyncCell;

pub fn run_suite() -> bool {
    let checks: [(&str, fn() -> Result<(), &'static str>); 4] = [
        ("pci_scan", test_pci_scan),
        ("untyped_alloc", test_untyped_alloc),
        ("mmio_user", test_mmio_user),
        ("irq_relay", test_irq_relay),
    ];
    let mut passed = 0u32;
    for (name, test) in checks {
        match test() {
            Ok(()) => {
                passed += 1;
                write_marker(format_args!("m5:test:{name}: PASS"));
            }
            Err(reason) => {
                error!("m5", "test {name} failed: {reason}");
                write_marker(format_args!("m5:test:{name}: FAIL ({reason})"));
            }
        }
    }
    let total = checks.len() as u32;
    if passed == total {
        write_marker(format_args!("m5: RESULT PASS ({passed}/{total})"));
        true
    } else {
        write_marker(format_args!("m5: RESULT FAIL ({passed}/{total})"));
        false
    }
}

// ---- shared ring-3 window (own VAs; the m4 window is long torn down) -----

const M5_CODE_VA: u64 = 0x0000_0000_0051_0000;
const M5_DATA_VA: u64 = 0x0000_0000_0051_1000;
const M5_STACK_VA: u64 = 0x0000_0000_7FD0_0000;
const M5_STACK_TOP: u64 = M5_STACK_VA + 4096;
const M5_REGIONS: [(u64, u64); 3] = [
    (M5_CODE_VA, M5_CODE_VA + 4096),
    (M5_DATA_VA, M5_DATA_VA + 4096),
    (M5_STACK_VA, M5_STACK_TOP),
];

/// The word the untyped payload stamps through its self-mapped window.
const M5_MAGIC: u64 = 0xDEAD_BEEF_CAFE_0051;

/// Payload exit codes (the diagnostic channel — see each builder).
const EXIT_OK: u32 = 42;

// ---- the payload emitter (self-contained; suites do not share helpers) ----

/// A tiny hand-assembled ring-3 program buffer. Encodings are the same
/// x86-64 facts the m3/m4 emitters use, re-derived here because the
/// suites are deliberately independent.
struct P {
    b: [u8; 1024],
    n: usize,
}

impl P {
    fn new() -> Self {
        Self { b: [0; 1024], n: 0 }
    }
    fn emit(&mut self, bytes: &[u8]) {
        self.b[self.n..self.n + bytes.len()].copy_from_slice(bytes);
        self.n += bytes.len();
    }
    fn emit_u32(&mut self, v: u32) {
        self.emit(&v.to_le_bytes());
    }
    fn here(&self) -> usize {
        self.n
    }
    /// mov eax, imm32 (syscall number / thread_exit verb).
    fn mov_eax(&mut self, v: u32) {
        self.emit(&[0xB8]);
        self.emit_u32(v);
    }
    /// mov edi, imm32 (arg 0).
    fn mov_edi(&mut self, v: u32) {
        self.emit(&[0xBF]);
        self.emit_u32(v);
    }
    /// mov esi, imm32 (arg 1).
    fn mov_esi(&mut self, v: u32) {
        self.emit(&[0xBE]);
        self.emit_u32(v);
    }
    /// movabs reg, imm64 — reg ≥ 8 takes the REX.B form (0x49).
    fn movabs(&mut self, reg: u8, v: u64) {
        let rex = if reg >= 8 { 0x49 } else { 0x48 };
        self.emit(&[rex, 0xB8 | (reg & 7)]);
        self.emit(&v.to_le_bytes());
    }
    /// syscall (0F 05).
    fn syscall_(&mut self) {
        self.emit(&[0x0F, 0x05]);
    }
    /// test rax, rax (48 85 C0).
    fn test_rax_rax(&mut self) {
        self.emit(&[0x48, 0x85, 0xC0]);
    }
    /// cmp rax, rdx (48 39 D0).
    fn cmp_rax_rdx(&mut self) {
        self.emit(&[0x48, 0x39, 0xD0]);
    }
    /// cmp rax, imm8 sign-extended (48 83 F8 ib).
    fn cmp_rax_i8(&mut self, v: i8) {
        self.emit(&[0x48, 0x83, 0xF8, v as u8]);
    }
    /// mov rbx, rax (48 89 C3).
    fn mov_rbx_rax(&mut self) {
        self.emit(&[0x48, 0x89, 0xC3]);
    }
    /// mov rcx, rax (48 89 C1).
    fn mov_rcx_rax(&mut self) {
        self.emit(&[0x48, 0x89, 0xC1]);
    }
    /// dec rcx (48 FF C9).
    fn dec_rcx(&mut self) {
        self.emit(&[0x48, 0xFF, 0xC9]);
    }
    /// mov [rbx], rax (48 89 03).
    fn store_rax_at_rbx(&mut self) {
        self.emit(&[0x48, 0x89, 0x03]);
    }
    /// mov [rbx+disp8], rax (48 89 43 dd).
    fn store_rax_at_rbx_d8(&mut self, d: i8) {
        self.emit(&[0x48, 0x89, 0x43, d as u8]);
    }
    /// mov [rcx], rdx (48 89 11).
    fn store_rdx_at_rcx(&mut self) {
        self.emit(&[0x48, 0x89, 0x11]);
    }
    /// mov rax, [rcx] (48 8B 01).
    fn load_rax_from_rcx(&mut self) {
        self.emit(&[0x48, 0x8B, 0x01]);
    }
    /// mov rax, [rbx+disp32] (48 8B 83 dd dd dd dd).
    fn load_rax_from_rbx_d32(&mut self, d: i32) {
        self.emit(&[0x48, 0x8B, 0x83]);
        self.emit_u32(d as u32);
    }
    /// mov [r8], rax (49 89 00).
    fn store_rax_at_r8(&mut self) {
        self.emit(&[0x49, 0x89, 0x00]);
    }
    /// mov [r8+disp8], rax (49 89 40 dd).
    fn store_rax_at_r8_d8(&mut self, d: i8) {
        self.emit(&[0x49, 0x89, 0x40, d as u8]);
    }
    /// mov rdx, [r8] (49 8B 10).
    fn load_rdx_from_r8(&mut self) {
        self.emit(&[0x49, 0x8B, 0x10]);
    }
    /// jle rel8 — returns the hole index to patch.
    fn jle8_hole(&mut self) -> usize {
        self.emit(&[0x7E, 0]);
        self.n - 1
    }
    /// jnz/jbe rel8 (ZF and CF variants share the emitter shape).
    fn jnz8_hole(&mut self) -> usize {
        self.emit(&[0x75, 0]);
        self.n - 1
    }
    /// jbe rel8.
    fn jbe8_hole(&mut self) -> usize {
        self.emit(&[0x76, 0]);
        self.n - 1
    }
    /// Patch a rel8 hole: displacement is relative to the byte AFTER it.
    fn patch8(&mut self, hole: usize, target: usize) {
        let rel = target as isize - (hole + 1) as isize;
        assert!((-128..=127).contains(&rel), "rel8 out of range");
        self.b[hole] = rel as u8;
    }
    /// mov eax, 2; mov edi, code; syscall — SYS_THREAD_EXIT(code).
    fn exit_with(&mut self, code: u32) {
        self.mov_eax(2);
        self.mov_edi(code);
        self.syscall_();
    }
}

/// Fixed-capacity forward-jump table: (hole offset, exit code).
type Holes = [(usize, u32); 8];

/// Emit the fail stubs and patch every recorded hole at its own stub.
fn emit_fail_stubs(p: &mut P, holes: &Holes, count: usize) {
    for (hole, code) in holes.iter().take(count) {
        let target = p.here();
        p.patch8(*hole, target);
        p.exit_with(*code);
    }
}

// ---- shared ring-3 scaffold (the m4 process-window pattern) ---------------

/// Map the code/data/stack window into a process root, copy the payload,
/// zero data+stack. Returns the data page's physical frame (the kernel
/// reads payload results back through its direct-map alias) and the free
/// frame count after setup.
///
/// # Safety
/// IF=0; `root` a live, owned PML4; every frame freshly allocated and
/// written only through its direct-map alias.
unsafe fn m5_setup(root: u64, code: &[u8]) -> Result<(u64, u64), &'static str> {
    let mut phys = [0u64; 3];
    for slot in phys.iter_mut() {
        *slot = frames::alloc().ok_or("frame exhaustion for the m5 window")?;
    }
    // SAFETY: caller contract; W^X pairs — code RX, data/stack RW+NX.
    unsafe {
        paging::map_user_page_4k(root, M5_CODE_VA, phys[0], false, true)
            .map_err(|_| "m5 code page map failed")?;
        paging::map_user_page_4k(root, M5_DATA_VA, phys[1], true, false)
            .map_err(|_| "m5 data page map failed")?;
        paging::map_user_page_4k(root, M5_STACK_VA, phys[2], true, false)
            .map_err(|_| "m5 stack page map failed")?;
        core::ptr::copy_nonoverlapping(
            code.as_ptr(),
            (phys[0] + paging::KERNEL_OFFSET) as *mut u8,
            code.len(),
        );
        core::ptr::write_bytes((phys[1] + paging::KERNEL_OFFSET) as *mut u8, 0, 4096);
        core::ptr::write_bytes((phys[2] + paging::KERNEL_OFFSET) as *mut u8, 0, 4096);
    }
    Ok((phys[1], frames::free_frames()))
}

/// The shared m5 user-thread entry: register the window, then iretq into
/// ring 3.
fn m5_thread_entry(_arg: usize) {
    crate::sync::without_interrupts(|| {
        sched::set_current_user_regions(&M5_REGIONS).expect("m5 user regions rejected");
        // SAFETY: the window pages are mapped U/S in this process's space
        // by m5_setup; RIP/RSP are canonical and inside the registered
        // regions; RSP0/scratch describe this thread (programmed at
        // switch-in, re-checked by enter_user).
        unsafe { syscall::enter_user(M5_CODE_VA, M5_STACK_TOP) };
    })
}

/// Yield until only the bootstrap thread remains, then one more pass so
/// the last zombie is reaped (the m3/m4 drain discipline).
fn m5_drain(max_yields: usize) -> Result<usize, &'static str> {
    let mut used = 0;
    while sched::live_threads() > 1 {
        if used >= max_yields {
            return Err("threads still live after the yield bound (scheduler stuck?)");
        }
        sched::yield_now();
        used += 1;
    }
    sched::yield_now();
    Ok(used + 1)
}

/// Kernel-side read of a payload result word: the data page through its
/// direct-map alias (a supervisor mapping — no SMAP bracket needed; the
/// ring-3 writes landed in the same physical frame).
///
/// # Safety
/// IF=0; `data_phys` is m5_setup's owned data frame.
unsafe fn data_word(data_phys: u64, off: u64) -> u64 {
    // SAFETY: caller contract; aligned u64 read of a direct-mapped frame.
    unsafe { core::ptr::read_volatile((data_phys + off + paging::KERNEL_OFFSET) as *const u64) }
}

/// A bounded pause — enough host-independent iterations that a ~14 MHz
/// HPET counter provably advances, without any timing assumptions.
fn spin_bounded(iters: u32) {
    for _ in 0..iters {
        core::hint::spin_loop();
    }
}

// ---- 1. pci_scan: the kernel's bus-0 walk found and armed the fixture ----

/// The virtio-blk device IDs the reference QEMU fixture can present:
/// 0x1001 (transitional — QEMU's default) or 0x1042 (modern-only).
const VIRTIO_BLK_TRANSITIONAL: u16 = 0x1001;
const VIRTIO_BLK_MODERN: u16 = 0x1042;

fn test_pci_scan() -> Result<(), &'static str> {
    if pci::pci_count() == 0 {
        return Err("nothing recorded on bus 0 — did the boot scan run at all?");
    }
    let Some(v) = pci::find_virtio(pci::VIRTIO_TYPE_BLOCK) else {
        return Err("no virtio-block function on bus 0 — is the scratch disk attached?");
    };
    if v.device_id != VIRTIO_BLK_TRANSITIONAL && v.device_id != VIRTIO_BLK_MODERN {
        return Err("virtio-block device ID is neither transitional 0x1001 nor modern 0x1042");
    }
    // The four structures a virtio 1.0 driver needs, each resolved to a
    // memory BAR (the notify multiplier may legitimately be 0 — a shared
    // doorbell — but the structure must exist).
    for c in [v.common, v.notify, v.isr, v.device_cfg] {
        if !c.present {
            return Err("virtio capability walk found no common/notify/isr/device structure");
        }
        if c.length == 0 {
            return Err("a virtio structure has zero length");
        }
    }
    // The common config must cover the virtio 1.0 layout (56 bytes).
    if v.common.length < 56 {
        return Err("common config shorter than the virtio 1.0 layout");
    }
    if !v.msix.present || v.msix.table_size == 0 {
        return Err("virtio-blk has no usable MSI-X capability — the relay plan needs it");
    }
    let Some(f) = pci::pci_function(v.pci_index) else {
        return Err("recorded function vanished from the table");
    };
    for c in [v.common, v.notify, v.isr, v.device_cfg] {
        let bi = c.bar as usize;
        if bi > 5 || f.bar_is_io[bi] || f.bar_base[bi] == 0 {
            return Err("a virtio structure points at an absent or IO BAR");
        }
        if f.bar_size[bi] < u64::from(c.offset) + u64::from(c.length) {
            return Err("a virtio structure extends past its BAR");
        }
    }
    if v.msix.table_bar > 5
        || f.bar_is_io[v.msix.table_bar as usize]
        || f.bar_base[v.msix.table_bar as usize] == 0
    {
        return Err("MSI-X table points at an absent or IO BAR");
    }
    // The kernel-policy write took: MEM|BUS MASTER readable back from
    // config space (a re-read, not the cached record).
    let Some(cmd) = pci::config_read16_of(v.pci_index, 0x04) else {
        return Err("recorded function's config space unreadable");
    };
    if cmd & 0x6 != 0x6 {
        return Err("MEM|BUS MASTER did not stick in the command register");
    }
    info!(
        "m5",
        "pci_scan: virtio-blk at 00:{:02}.{} device_id {:#06x} ({}) — common bar{}+{:#x} len {:#x}, notify bar{}+{:#x} mult {:#x}, isr bar{}+{:#x}, device bar{}+{:#x}, msix {} entries table bar{}+{:#x}; command {:#06x} (MEM|BUS MASTER by kernel policy); {} function(s) on bus 0",
        v.dev,
        v.func,
        v.device_id,
        if v.transitional {
            "transitional"
        } else {
            "modern"
        },
        v.common.bar,
        v.common.offset,
        v.common.length,
        v.notify.bar,
        v.notify.offset,
        v.notify_off_multiplier,
        v.isr.bar,
        v.isr.offset,
        v.device_cfg.bar,
        v.device_cfg.offset,
        v.msix.table_size,
        v.msix.table_bar,
        v.msix.table_offset,
        cmd,
        pci::pci_count(),
    );
    Ok(())
}

// ---- 2. untyped_alloc: owned frames through ring 3 -------------------------

/// The payload: alloc slot 0 and slot 1 (both must pay a positive phys),
/// self-map slot 0 writable (positive VA), stamp and read back the magic
/// word through the window, prove the CONSUMED slot 0 re-map refuses with
/// -2, exit 42. Fail exits: 43 = an alloc refused, 44 = the map refused,
/// 45 = the window read-back mismatched, 46 = the consumed slot re-mapped.
fn build_untyped_payload() -> P {
    let mut p = P::new();
    let mut holes: Holes = [(0, 0); 8];
    let mut hn = 0usize;
    p.movabs(3, M5_DATA_VA); // rbx = the data page
    // alloc frame #1 → slot 0
    p.mov_eax(syscall::SYS_ALLOC_FRAME as u32);
    p.mov_edi(0);
    p.syscall_();
    p.test_rax_rax();
    holes[hn] = (p.jle8_hole(), 43);
    hn += 1;
    p.store_rax_at_rbx(); // data[0] = phys1
    // alloc frame #2 → slot 1
    p.mov_eax(syscall::SYS_ALLOC_FRAME as u32);
    p.mov_edi(1);
    p.syscall_();
    p.test_rax_rax();
    holes[hn] = (p.jle8_hole(), 43);
    hn += 1;
    p.store_rax_at_rbx_d8(8); // data[8] = phys2
    // self-map slot 0, writable
    p.mov_eax(syscall::SYS_MAP_MEMORY as u32);
    p.mov_edi(0);
    p.mov_esi(1);
    p.syscall_();
    p.test_rax_rax();
    holes[hn] = (p.jle8_hole(), 44);
    hn += 1;
    p.store_rax_at_rbx_d8(16); // data[16] = window va
    p.mov_rcx_rax();
    p.movabs(2, M5_MAGIC);
    p.store_rdx_at_rcx(); // [va] = magic
    p.load_rax_from_rcx(); // rax = [va]
    p.store_rax_at_rbx_d8(24); // data[24] = read-back
    p.cmp_rax_rdx();
    holes[hn] = (p.jnz8_hole(), 45);
    hn += 1;
    // the map CONSUMED slot 0 — re-mapping the empty slot must refuse -2
    p.mov_eax(syscall::SYS_MAP_MEMORY as u32);
    p.mov_edi(0);
    p.mov_esi(1);
    p.syscall_();
    p.cmp_rax_i8(-2);
    holes[hn] = (p.jnz8_hole(), 46);
    hn += 1;
    p.exit_with(EXIT_OK);
    emit_fail_stubs(&mut p, &holes, hn);
    p
}

fn test_untyped_alloc() -> Result<(), &'static str> {
    let baseline = frames::free_frames();
    let pid = proc::create("m5-untyped")?;
    let root = proc::pml4_of(pid).ok_or("process lost its pml4")?;
    let p = build_untyped_payload();
    // SAFETY: m5_setup's contract (IF=0 suite discipline, owned root,
    // fresh frames, direct-map writes).
    let (dphys, _after_setup) = unsafe { m5_setup(root, &p.b[..p.n])? };

    let tid = sched::spawn_in_proc("m5-untyp", m5_thread_entry, 0, pid)?;
    sched::yield_now();
    m5_drain(128)?;

    let Some(status) = syscall::exit_status_of(tid) else {
        return Err("no thread_exit recorded for the untyped payload");
    };
    if status != u64::from(EXIT_OK) {
        return Err(match status {
            43 => "an alloc_frame refused (43)",
            44 => "the self-map refused (44)",
            45 => "the window read-back mismatched (45)",
            46 => "re-mapping the consumed slot did not refuse -2 (46)",
            _ => "the payload exited with a code from nowhere in the contract",
        });
    }
    // SAFETY: IF=0; dphys is m5_setup's owned, direct-mapped data frame.
    let (phys1, phys2, va, readback) = unsafe {
        (
            data_word(dphys, 0),
            data_word(dphys, 8),
            data_word(dphys, 16),
            data_word(dphys, 24),
        )
    };
    if phys1 == 0 || phys1 % 4096 != 0 || phys2 == 0 || phys2 % 4096 != 0 {
        return Err("alloc_frame paid a non-page-aligned or zero phys");
    }
    if phys1 == phys2 {
        return Err("two allocations paid the SAME frame (allocator double-sold)");
    }
    if va == 0 || va >= 0x0000_8000_0000_0000 {
        return Err("the kernel-chosen window VA is not lower-half");
    }
    if readback != M5_MAGIC {
        return Err("the ring-3 stamp did not survive the window round-trip");
    }
    // The consumed slot is empty; slot 1 still holds the OWNED frame.
    if cap::read(pid, 0).is_ok() {
        return Err("the consumed untyped cap survived its map");
    }
    let c1 = cap::read(pid, 1).map_err(|_| "slot 1's untyped cap vanished")?;
    match c1.obj {
        CapObj::Untyped { phys } if phys == phys2 => {}
        _ => return Err("slot 1 does not hold the second frame"),
    }
    // The ownership promise: destroying an owned cap returns exactly one
    // frame to the allocator.
    let before_destroy = frames::free_frames();
    cap::destroy(pid, 1).map_err(|_| "owned-frame destroy refused")?;
    if frames::free_frames() != before_destroy + 1 {
        return Err("destroying the untyped cap did not return exactly one frame");
    }
    // Teardown: window frames, page tables, AND the mapped first frame
    // all come back through proc::destroy's walk.
    proc::destroy(pid)?;
    let after = frames::free_frames();
    if after != baseline {
        return Err("untyped demo teardown is not frame-exact");
    }
    info!(
        "m5",
        "untyped_alloc: ring 3 allocated two owned frames ({phys1:#x}, {phys2:#x}), self-mapped the first at {va:#x} writable (cap consumed — the re-map refused -2 in ring 3), stamped and read back {readback:#x} through the window; the kernel destroyed the second cap (+1 frame exactly) and proc::destroy reclaimed the mapped frame — frames {after} (teardown exact)"
    );
    Ok(())
}

// ---- 3. mmio_user: device registers under ring-3 eyes ----------------------

/// The payload: self-map slot 0 (the kernel-minted HPET Mmio cap)
/// READ-ONLY, read the main counter at +0xF0, spin a bounded delay, read
/// again, and exit 42 only if the second read is strictly greater.
/// Fail exits: 53 = the map refused, 54 = the counter did not advance.
fn build_mmio_payload() -> P {
    let mut p = P::new();
    let mut holes: Holes = [(0, 0); 8];
    let mut hn = 0usize;
    p.mov_eax(syscall::SYS_MAP_MEMORY as u32);
    p.mov_edi(0);
    p.mov_esi(0); // read-only
    p.syscall_();
    p.test_rax_rax();
    holes[hn] = (p.jle8_hole(), 53);
    hn += 1;
    p.mov_rbx_rax(); // rbx = the HPET window (callee-saved per ABI v1)
    // r8/r9/r10 are CALLER-saved across syscalls (ADR-0017) — the data
    // page pointer is loaded into r8 only AFTER the last fallible call.
    p.movabs(8, M5_DATA_VA); // r8 = the data page
    p.load_rax_from_rbx_d32(0xF0); // t1 = main counter (low dword path)
    p.store_rax_at_r8(); // data[0] = t1
    p.movabs(1, 100_000); // bounded delay — no timing assumptions
    let loop_at = p.here();
    p.dec_rcx();
    let back = p.jnz8_hole();
    p.patch8(back, loop_at);
    p.load_rax_from_rbx_d32(0xF0); // t2
    p.store_rax_at_r8_d8(8); // data[8] = t2
    p.load_rdx_from_r8(); // rdx = t1
    p.cmp_rax_rdx();
    holes[hn] = (p.jbe8_hole(), 54); // t2 <= t1 → fail
    hn += 1;
    p.exit_with(EXIT_OK);
    emit_fail_stubs(&mut p, &holes, hn);
    p
}

fn test_mmio_user() -> Result<(), &'static str> {
    // Kernel-side first: the main counter must actually run before ring 3
    // is asked to observe it (ADR-0005 — no decorative assertions).
    let hpet_va = paging::mmio_alias_va(intc::HPET_PHYS);
    // SAFETY: ring 0, IF=0 suite discipline; hpet_va is the mapped MMIO
    // alias (the same one entry.rs programs the legacy-replacement bit
    // through).
    let conf = unsafe { intc::hpet_read(hpet_va, intc::HPET_GEN_CONF) };
    if conf & intc::HPET_ENABLE_CNF == 0 {
        // SAFETY: as above; read-modify-write of the enable bit only.
        unsafe {
            intc::hpet_write(hpet_va, intc::HPET_GEN_CONF, conf | intc::HPET_ENABLE_CNF);
        };
    }
    // SAFETY: as above.
    let conf2 = unsafe { intc::hpet_read(hpet_va, intc::HPET_GEN_CONF) };
    if conf2 & intc::HPET_ENABLE_CNF == 0 {
        return Err("the HPET main counter refuses to run (ENABLE_CNF did not stick)");
    }
    // SAFETY: as above.
    let k_t0 = unsafe { intc::hpet_read(hpet_va, intc::HPET_MAIN_COUNTER) };
    spin_bounded(200_000);
    // SAFETY: as above.
    let k_t1 = unsafe { intc::hpet_read(hpet_va, intc::HPET_MAIN_COUNTER) };
    if k_t1 == k_t0 {
        return Err("the HPET main counter stands still in the kernel's own observation");
    }

    let baseline = frames::free_frames();
    let pid = proc::create("m5-mmio")?;
    let root = proc::pml4_of(pid).ok_or("process lost its pml4")?;
    let p = build_mmio_payload();
    // SAFETY: m5_setup's contract.
    let (dphys, _after_setup) = unsafe { m5_setup(root, &p.b[..p.n])? };
    // The kernel-minted Mmio cap — the ONLY way ring 3 ever sees device
    // registers (ADR-0021): READ only, one page, over the counter block.
    let slot = cap::grant(
        pid,
        Cap {
            obj: CapObj::Mmio {
                phys: intc::HPET_PHYS,
                pages: 1,
            },
            rights: cap::RIGHTS_READ,
        },
    )
    .map_err(|_| "mmio cap grant failed")?;
    if slot != 0 {
        return Err("the mmio cap did not land in slot 0 (payload contract)");
    }

    let tid = sched::spawn_in_proc("m5-mmio", m5_thread_entry, 0, pid)?;
    sched::yield_now();
    m5_drain(128)?;

    let Some(status) = syscall::exit_status_of(tid) else {
        return Err("no thread_exit recorded for the mmio payload");
    };
    if status != u64::from(EXIT_OK) {
        return Err(match status {
            53 => "the read-only mmio self-map refused (53)",
            54 => "the counter did not advance between the ring-3 reads (54)",
            _ => "the payload exited with a code from nowhere in the contract",
        });
    }
    // SAFETY: IF=0; dphys is m5_setup's owned data frame.
    let (rt1, rt2) = unsafe { (data_word(dphys, 0), data_word(dphys, 8)) };
    if rt2 <= rt1 {
        return Err("ring-3 counter reads are not increasing (kernel-side check)");
    }
    // The Mmio cap SURVIVES the mapping (descriptive, never consumed).
    let c = cap::read(pid, 0).map_err(|_| "the mmio cap vanished on mapping")?;
    match c.obj {
        CapObj::Mmio { phys, pages } if phys == intc::HPET_PHYS && pages == 1 => {}
        _ => return Err("slot 0 no longer holds the HPET Mmio cap"),
    }
    // Teardown must free exactly the RAM: the walk SKIPS the MMIO leaf
    // (the paging guard of ADR-0021 — without it this destroy would try
    // to hand device registers to the frame allocator and halt).
    let before_destroy = frames::free_frames();
    let walk_freed = proc::destroy(pid)?;
    let after = frames::free_frames();
    if after != baseline {
        error!(
            "m5",
            "mmio teardown accounting: baseline {baseline}, before destroy {before_destroy}, walk freed {walk_freed}, after {after}"
        );
        return Err("mmio demo teardown is not frame-exact (did the walk free a device page?)");
    }
    info!(
        "m5",
        "mmio_user: ring 3 read the HPET main counter through a kernel-minted Mmio cap — {rt1:#x} → {rt2:#x} strictly increasing across a bounded delay (kernel observed {k_t0:#x} → {k_t1:#x}), read-only window, cap survived the mapping, teardown freed exactly the RAM (device page skipped) — frames {after}"
    );
    Ok(())
}

// ---- 4. irq_relay: interrupts become notifications -------------------------

/// The relay vector this test arms (first of the 48..63 range).
const RELAY_VEC: u64 = 48;
/// A relay vector deliberately left unregistered for the spurious test.
const SPURIOUS_VEC: u64 = 49;
/// The badge the relay delivers (the waiter's expected word).
const RELAY_BADGE: u64 = 0x51AB;
// The IPI is addressed to THIS CPU's own APIC ID (physical destination,
// fixed delivery) rather than the ICR "self" shorthand: the shorthand is
// not honored by QEMU's TCG APIC model, while an addressed IPI walks the
// real delivery path and lands on the same CPU — which is all the relay
// test needs (a device MSI would arrive exactly the same way).

/// What the parked waiter received (0 until it wakes).
static RELAY_SEEN: SyncCell<u64> = SyncCell::new(0);

/// The waiter thread: blocks in `ipc::wait` until the relay delivery
/// wakes it, recording the badge (u64::MAX on a wait error).
fn relay_waiter(arg: usize) {
    match ipc::wait(arg as u32) {
        Ok(badge) => {
            // SAFETY: single writer (this thread), IF-agnostic word store.
            unsafe { *RELAY_SEEN.get() = badge };
        }
        Err(_) => {
            // SAFETY: as above.
            unsafe { *RELAY_SEEN.get() = u64::MAX };
        }
    }
}

fn test_irq_relay() -> Result<(), &'static str> {
    // Registration seams first — pure table logic, no hardware.
    if relay::register(47, 0, RELAY_BADGE).is_ok() {
        return Err("register accepted a vector outside the relay range");
    }
    if relay::register(64, 0, RELAY_BADGE).is_ok() {
        return Err("register accepted a vector outside the relay range");
    }
    let nid = ipc::create_notification().map_err(|_| "notification table full")?;
    if relay::register(RELAY_VEC, nid, 0).is_ok() {
        return Err("register accepted the empty badge");
    }
    relay::register(RELAY_VEC, nid, RELAY_BADGE).map_err(|_| "first registration refused")?;
    if !relay::registered(RELAY_VEC) {
        return Err("registered() disagrees with a successful register");
    }
    if relay::register(RELAY_VEC, nid, RELAY_BADGE).is_ok() {
        return Err("double registration accepted (would strand the old waiter)");
    }

    // Park a kernel waiter on the notification, then fire the LIVE path:
    // a LAPIC self-IPI at the relay vector. The stub EOIs both controllers
    // and hands the vector to relay::handle → ipc::notify → the waiter
    // wakes with the badge. This is the exact chain a virtio MSI-X
    // interrupt walks from M5.2 on (only the doorbell differs: the device
    // writes the MSI address instead of the ICR).
    let threads0 = sched::live_threads();
    let _wid = sched::spawn("relay-wait", relay_waiter, nid as usize)?;
    let n0 = ipc::stats().notifies;
    sched::yield_now(); // the waiter parks in ipc::wait FIRST

    let lapic_va = paging::mmio_alias_va(paging::apic_base_phys());
    // SAFETY: ring 0, IF=0 here; the LAPIC alias is mapped (entry.rs
    // programs through the same alias). The self-IPI stays pending until
    // IF=1 — the yields below run threads interruptible, so delivery
    // happens on the next scheduler pass.
    // SAFETY: as above; LAPIC_ID's xAPIC destination field is bits 31:24.
    let apic_id = unsafe { intc::lapic_read(lapic_va, intc::LAPIC_ID) >> 24 };
    unsafe {
        intc::lapic_write(lapic_va, intc::LAPIC_ICR_HI, apic_id << 24);
        intc::lapic_write(lapic_va, intc::LAPIC_ICR_LO, RELAY_VEC as u32);
    }
    // The IPI latches into the LAPIC's IRR but is only TAKEN while this
    // CPU runs with IF=1 — and the boot thread masks interrupts through
    // the suites (observed live: irr1 bit 16 set, zero stub deliveries).
    // Open a bounded IF=1 window until the stub records the delivery
    // (timer ticks flow in the same window — harmless), then restore the
    // prior masking: the woken waiter runs in the yields below either
    // way. A device MSI-X interrupt in M5.2 needs no such window — it
    // arrives while the driver process runs interruptible in ring 3.
    let if_before = crate::arch::x86_64::interrupts_enabled();
    crate::arch::x86_64::sti();
    let mut spins = 0u32;
    while relay::delivery_count(RELAY_VEC) == 0 && spins < 50 {
        spin_bounded(20_000);
        spins += 1;
    }
    if !if_before {
        crate::arch::x86_64::cli();
    }
    if let Err(e) = m5_drain(64) {
        // Diagnostic before the verdict: was the vector delivered at all,
        // did the stub run, did the notify land? (IRR1/ISR1 cover
        // vectors 32..63 — bit 16 is vector 48.)
        // SAFETY: ring 0, IF=0; the LAPIC alias is mapped.
        let (irr, isr) = unsafe {
            (
                intc::lapic_read(lapic_va, intc::LAPIC_IRR1),
                intc::lapic_read(lapic_va, intc::LAPIC_ISR1),
            )
        };
        error!(
            "m5",
            "relay drain failed ({e}): stub deliveries={}, notifies={}, irr1={irr:#x}, isr1={isr:#x}",
            relay::delivery_count(RELAY_VEC),
            ipc::stats().notifies - n0,
        );
        return Err("the relay waiter never woke (see the accounting line above)");
    }

    // SAFETY: IF=0; the waiter stored its result before exiting and the
    // drain reaped it.
    let seen = unsafe { *RELAY_SEEN.get() };
    if seen != RELAY_BADGE {
        return Err("the waiter did not wake with the relay's badge");
    }
    if relay::delivery_count(RELAY_VEC) != 1 {
        return Err("the relay stub delivered the vector not exactly once");
    }
    if ipc::stats().notifies - n0 != 1 {
        return Err("relay delivery did not produce exactly one notify");
    }
    if sched::live_threads() != threads0 {
        return Err("the waiter thread was not reaped");
    }
    // (A kernel thread that RETURNS leaves no thread_exit record — the
    // reap above is the evidence; `wid` needs no exit-status check.)

    // Spurious + release seams: an unregistered delivery is counted and
    // survived (never fatal in interrupt context), release is one-shot.
    relay::handle(SPURIOUS_VEC);
    if relay::delivery_count(SPURIOUS_VEC) != 1 {
        return Err("spurious delivery not counted");
    }
    relay::release(RELAY_VEC).map_err(|_| "release of a live relay refused")?;
    if relay::registered(RELAY_VEC) {
        return Err("registered() disagrees with a successful release");
    }
    if relay::release(RELAY_VEC).is_ok() {
        return Err("second release accepted");
    }
    ipc::destroy_notification(nid).map_err(|_| "notification teardown refused")?;

    info!(
        "m5",
        "irq_relay: vector {RELAY_VEC} → notification {nid} badge {RELAY_BADGE:#x} — a LAPIC self-IPI walked the live stub (dual EOI) into ipc::notify and woke the parked waiter with the exact badge (1 delivery, 1 notify, thread reaped); a spurious hit on unregistered vector {SPURIOUS_VEC} was counted and survived; register/release seams hold (range, empty badge, double-register, double-release all refused)"
    );
    Ok(())
}
