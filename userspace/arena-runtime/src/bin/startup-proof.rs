#![no_std]
#![no_main]

extern crate alloc;

use alloc::alloc::{alloc, dealloc};
use arena_lib::abi::{
    RIGHTS_DESTROY as ABI_RIGHT_DESTROY, RIGHTS_READ as ABI_RIGHT_READ, SYS_CAP_COPY,
    SYS_CAP_DESCRIBE, SYS_CAP_DESTROY, SYS_TIMER_ARM, SYS_TLS_SET, SYS_WAIT, syscall1, syscall2,
    syscall3, syscall6,
};
use arena_platform_core::startup::{
    CAP_KIND_NOTIFICATION, CapabilityRole, RIGHT_COPY, RIGHT_DESTROY, RIGHT_READ, RIGHT_WRITE,
    StartupView,
};
use arena_runtime::{
    heap::BoundedHeap,
    startup::enter,
    tls::{self, ThreadControlBlock},
};
use core::alloc::Layout;
use core::panic::PanicInfo;

#[global_allocator]
static NATIVE_HEAP: BoundedHeap = BoundedHeap::new();

const APP_ID: &[u8] = b"com.arena.startup";
const ENTRY: u64 = 0x0020_0000;
const INITIAL_STACK_TOP: u64 = 0x0021_4000;
const EXIT_OK: u64 = 37;
const EXIT_APP_CALLED_ON_BAD_CAP: u64 = 38;
const EXIT_HEAP_FAILED: u64 = 42;
const EXIT_TLS_FAILED: u64 = 43;
const TLS_SENTINEL: u64 = 0xA11C_EA05_F512_0042;
const HEAP_TEST_BYTES: usize = arena_runtime::heap::MAX_ALLOCATION_BYTES;
const HEAP_TEST_PAGES: usize = arena_runtime::heap::MAX_HEAP_PAGES;
const HEAP_CAP_SLOT: u64 = arena_runtime::heap::HEAP_CAP_SLOT;

#[unsafe(no_mangle)]
static mut INITIAL_RSP: u64 = 0;
#[unsafe(no_mangle)]
static mut INITIAL_RFLAGS: u64 = 0;

#[repr(align(16))]
struct ReadOnlyTlsProbe([u8; 16]);
static READ_ONLY_TLS_PROBE: ReadOnlyTlsProbe = ReadOnlyTlsProbe(*b"read-only-proof!");

#[unsafe(no_mangle)]
static mut TLS_BLOCK: ThreadControlBlock = ThreadControlBlock::new();

#[unsafe(naked)]
#[unsafe(no_mangle)]
#[unsafe(link_section = ".text._start")]
pub extern "C" fn _start() -> ! {
    core::arch::naked_asm!(
        "mov qword ptr [rip + {initial_rsp}], rsp",
        "pushfq",
        "pop rax",
        "mov qword ptr [rip + {initial_rflags}], rax",
        "call {main}",
        "ud2",
        initial_rsp = sym INITIAL_RSP,
        initial_rflags = sym INITIAL_RFLAGS,
        main = sym startup_main,
    )
}

#[unsafe(no_mangle)]
extern "C" fn startup_main() -> ! {
    enter(check_startup)
}

fn tls_proof() -> bool {
    let block_pointer = core::ptr::addr_of_mut!(TLS_BLOCK);
    let Some(block) = core::ptr::NonNull::new(block_pointer) else {
        return false;
    };
    // SAFETY: this is a private RW+NX static in the proof process's .bss and
    // outlives the only user thread.
    unsafe {
        core::ptr::addr_of_mut!((*block_pointer).application_word).write(TLS_SENTINEL);
    }
    // SAFETY: the TCB remains live for this single-thread proof process.
    if unsafe { tls::install(block) }.is_err() {
        return false;
    }
    let Some(installed) = tls::current() else {
        return false;
    };
    // SAFETY: `install` set FS.base to this static writable TCB.
    if installed != block || unsafe { installed.as_ref().application_word } != TLS_SENTINEL {
        return false;
    }

    // Negative controls: a read-only ELF page and a canonical kernel-half
    // address must be refused without replacing the currently installed base.
    let read_only = core::ptr::addr_of!(READ_ONLY_TLS_PROBE.0) as u64;
    // SAFETY: SYS_TLS_SET treats the value as a checked address, never as a
    // kernel pointer; all reserved syscall arguments are zero.
    let read_only_status = unsafe { syscall6(SYS_TLS_SET, read_only, 0, 0, 0, 0, 0) };
    // SAFETY: same; kernel-half addresses are refused by current-region/PTE
    // validation before WRMSR.
    let kernel_status = unsafe { syscall6(SYS_TLS_SET, 0xFFFF_8000_0000_0000, 0, 0, 0, 0, 0) };
    let Some(still_installed) = tls::current() else {
        return false;
    };
    // SAFETY: failed set calls must leave the existing TCB and value intact.
    if read_only_status >= 0
        || kernel_status >= 0
        || still_installed != block
        || unsafe { still_installed.as_ref().application_word } != TLS_SENTINEL
    {
        return false;
    }

    // Park this thread through a capability-gated one-shot timer. The kernel
    // scheduler runs another (kernel) thread and must restore this thread's
    // FS.base on wake, without using or disturbing the GS/swapgs pair.
    // SAFETY: slot 1 is a listed Notification with READ|WRITE in both proof
    // variants; TIMER_ARM uses WRITE and WAIT uses READ.
    let timer = unsafe { syscall6(SYS_TIMER_ARM, 1, 0x544C_5301, 30_000, 0, 0, 0) };
    if timer <= 0 {
        return false;
    }
    // SAFETY: slot 1 is the same held Notification; timer expiry wakes us.
    let badge = unsafe { syscall6(SYS_WAIT, 1, 0, 0, 0, 0, 0) };
    let Some(resumed) = tls::current() else {
        return false;
    };
    // SAFETY: scheduler restoration must preserve this same live TCB/value.
    if badge != 0x544C_5301
        || resumed != block
        || unsafe { resumed.as_ref().application_word } != TLS_SENTINEL
    {
        return false;
    }

    // Clear FS.base while the TCB is still mapped. Do not call current()
    // after clearing: with base zero, FS-relative reads intentionally fault.
    // SAFETY: zero is the documented no-TLS value.
    (unsafe { syscall6(SYS_TLS_SET, 0, 0, 0, 0, 0, 0) }) == 0
}

fn heap_proof(instance_slot: u16) -> bool {
    if instance_slot == 5 {
        let passed = heap_slot_collision_proof();
        if passed {
            arena_lib::abi::write_all(b"m12:startup:heap-slot-red: PASS\n");
        }
        return passed;
    }
    heap_capacity_proof()
}

fn heap_slot_collision_proof() -> bool {
    let copied_rights = ABI_RIGHT_READ | ABI_RIGHT_DESTROY;
    // The described slot-1 Notification is explicitly granted READ|COPY|DESTROY
    // only in this control. Attenuate it into the runtime-reserved slot 63.
    // SAFETY: source 1 is a listed live cap with COPY; destination 63 is empty.
    if unsafe { syscall3(SYS_CAP_COPY, 1, HEAP_CAP_SLOT, copied_rights) } != 0 {
        return false;
    }
    let mut before = [0u64; 3];
    // SAFETY: `before` is a writable stack range in the process's initial map.
    if unsafe { syscall2(SYS_CAP_DESCRIBE, HEAP_CAP_SLOT, before.as_mut_ptr() as u64) } != 0
        || before[0] != u64::from(CAP_KIND_NOTIFICATION)
        || before[2] != copied_rights
    {
        // SAFETY: the attenuated reference has DESTROY if it was created.
        let _ = unsafe { syscall1(SYS_CAP_DESTROY, HEAP_CAP_SLOT) };
        return false;
    }

    let Ok(layout) = Layout::from_size_align(64, 16) else {
        // SAFETY: release the test-created cap before the process exits.
        let _ = unsafe { syscall1(SYS_CAP_DESTROY, HEAP_CAP_SLOT) };
        return false;
    };
    // This must fail without replacing the existing cap or leaking the
    // temporary frame that SYS_ALLOC_FRAME tries to issue into slot 63.
    // SAFETY: the requested layout is supported by BoundedHeap.
    let allocation = unsafe { alloc(layout) };
    if !allocation.is_null() {
        // SAFETY: return an unexpected live allocation before checking state.
        unsafe { dealloc(allocation, layout) };
    }

    let mut after = [0u64; 3];
    // SAFETY: `after` is a writable stack range in the process's initial map.
    let unchanged =
        unsafe { syscall2(SYS_CAP_DESCRIBE, HEAP_CAP_SLOT, after.as_mut_ptr() as u64) == 0 }
            && after == before;
    // SAFETY: `before` proved the copied cap has DESTROY and still names slot 63.
    let discarded = unsafe { syscall1(SYS_CAP_DESTROY, HEAP_CAP_SLOT) } == 0;
    allocation.is_null() && unchanged && discarded
}

fn heap_capacity_proof() -> bool {
    let Ok(full_page) = Layout::from_size_align(HEAP_TEST_BYTES, 16) else {
        return false;
    };
    let mut allocations = [core::ptr::null_mut::<u8>(); HEAP_TEST_PAGES];
    for index in 0..allocations.len() {
        // SAFETY: the runtime allocator returns a block large enough for the
        // exact layout or null on its explicit 32-page bound.
        let pointer = unsafe { alloc(full_page) };
        if pointer.is_null() {
            for previous in allocations.iter().take(index).copied() {
                // SAFETY: each earlier pointer is a live allocation above.
                unsafe { dealloc(previous, full_page) };
            }
            return false;
        }
        let pattern = index as u8 ^ 0xA5;
        // SAFETY: the allocation spans exactly 4,080 writable bytes.
        unsafe { pointer.write_bytes(pattern, HEAP_TEST_BYTES) };
        allocations[index] = pointer;
    }

    // Capacity refusal must return null before requesting/mapping a 33rd page.
    // SAFETY: same valid layout as the preceding full-page allocations.
    let overflow = unsafe { alloc(full_page) };
    if !overflow.is_null() {
        // SAFETY: unexpected but valid allocation; return it during cleanup.
        unsafe { dealloc(overflow, full_page) };
        for allocation in allocations {
            // SAFETY: every array entry is a live allocation.
            unsafe { dealloc(allocation, full_page) };
        }
        return false;
    }

    for (index, pointer) in allocations.iter().copied().enumerate() {
        let pattern = index as u8 ^ 0xA5;
        // SAFETY: the first/last byte lie inside their corresponding live
        // 4,080-byte allocations.
        if unsafe { pointer.read_volatile() } != pattern
            || unsafe { pointer.add(HEAP_TEST_BYTES - 1).read_volatile() } != pattern
        {
            for allocation in allocations {
                // SAFETY: all entries remain live until the cleanup loop.
                unsafe { dealloc(allocation, full_page) };
            }
            return false;
        }
    }
    for allocation in allocations {
        // SAFETY: each pointer was returned above and has not yet been freed.
        unsafe { dealloc(allocation, full_page) };
    }

    let small = match Layout::from_size_align(64, 16) {
        Ok(layout) => layout,
        Err(_) => return false,
    };
    // Freed blocks must be reused without allocating another page.
    // SAFETY: this is a supported bounded layout.
    let reused = unsafe { alloc(small) };
    if reused.is_null() {
        return false;
    }
    // SAFETY: the pointer is the live allocation returned immediately above.
    unsafe { dealloc(reused, small) };
    true
}

fn check_startup(view: StartupView<'_>) -> u64 {
    // SAFETY: _start initializes each private BSS witness exactly once before
    // calling Rust; this proof image is single-threaded.
    let initial_rsp = unsafe { core::ptr::addr_of!(INITIAL_RSP).read_volatile() };
    // SAFETY: paired with the immediately preceding _start write.
    let initial_rflags = unsafe { core::ptr::addr_of!(INITIAL_RFLAGS).read_volatile() };
    if initial_rsp != INITIAL_STACK_TOP
        || initial_rflags & (1 << 9) == 0
        || initial_rflags & (1 << 10) != 0
        || &view.application_id()[..APP_ID.len()] != APP_ID
        || view.application_id()[APP_ID.len()..]
            .iter()
            .any(|byte| *byte != 0)
        || !matches!(view.instance_slot(), 4 | 5)
        || view.instance_generation() != 42
        || view.argument_count() != 2
        || view.argument(0) != Some(&b"startup-probe"[..])
        || view.argument(1) != Some(&b"alpha"[..])
        || view.environment_count() != 1
        || view.environment(0) != Some(&b"MODE=proof"[..])
        || view.capability_count() != 1
        || view.cwd_descriptor().is_some()
        || view.stdin_descriptor().is_some()
        || view.stdout_descriptor().is_some()
        || view.stderr_descriptor().is_some()
        || view.page_size() != 4096
        || view.entry() != ENTRY
        || view.load_base() != ENTRY
        || view.clock_us() != 987654
    {
        arena_lib::abi::write_all(b"m12:startup:application-data: FAIL\n");
        return 39;
    }
    let Some(descriptor) = view.capability(0) else {
        return 40;
    };
    let expected_rights = if view.instance_slot() == 5 {
        RIGHT_READ | RIGHT_WRITE | RIGHT_COPY | RIGHT_DESTROY
    } else {
        RIGHT_READ | RIGHT_WRITE
    };
    if descriptor.rights != expected_rights {
        arena_lib::abi::write_all(b"m12:startup:badcap: APPLICATION-RAN\n");
        return EXIT_APP_CALLED_ON_BAD_CAP;
    }
    if descriptor.slot != 1
        || descriptor.role != CapabilityRole::Other
        || descriptor.kind != CAP_KIND_NOTIFICATION
    {
        arena_lib::abi::write_all(b"m12:startup:descriptor: FAIL\n");
        return 41;
    }
    if !tls_proof() {
        arena_lib::abi::write_all(b"m12:startup:tls: FAIL\n");
        return EXIT_TLS_FAILED;
    }
    arena_lib::abi::write_all(b"m12:startup:tls: PASS\n");
    if !heap_proof(view.instance_slot()) {
        arena_lib::abi::write_all(b"m12:startup:heap: FAIL\n");
        return EXIT_HEAP_FAILED;
    }
    arena_lib::abi::write_all(b"m12:startup:heap: PASS\n");
    arena_lib::abi::write_all(b"m12:startup:application: PASS\n");
    EXIT_OK
}

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    arena_lib::abi::write_all(b"m12:startup:panic\n");
    unsafe { arena_lib::abi::syscall1(arena_lib::abi::SYS_THREAD_EXIT, 99) };
    loop {
        core::hint::spin_loop();
    }
}

// `EXIT_APP_CALLED_ON_BAD_CAP` is emitted only if runtime verification is
// bypassed and the application closure observes the hostile rights row.
