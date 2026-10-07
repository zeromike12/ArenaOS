#![no_std]
#![no_main]

extern crate alloc;

use alloc::{
    alloc::{alloc, dealloc},
    vec::Vec,
};
use arena_desktop::{
    app_client,
    client::{self, Client},
    files::Files,
    model::Event,
};
use arena_lib::abi::{
    CAP_KIND_VM_REGION, RIGHTS_DESTROY, RIGHTS_READ, RIGHTS_WRITE, STATUS_BAD_ADDRESS,
    STATUS_BAD_ARG, STATUS_BUSY, SYS_CAP_DESCRIBE, SYS_CAP_DESTROY, SYS_VM_COMMIT, SYS_VM_PROTECT,
    VM_PROT_EXEC, VM_PROT_READ, VM_PROT_WRITE, syscall1, syscall2, syscall6,
};
use arena_runtime::capabilities::HeldCapability;
use arena_runtime::heap::{HeapUsage, MAX_SCALABLE_HEAP_PAGES, PAGE_BYTES, ScalableHeap};
use arena_startup_abi::startup::StartupView;
use core::alloc::Layout;

#[global_allocator]
static APPLICATION_HEAP: ScalableHeap = ScalableHeap::new();

const APPLICATION_ID: [u8; 32] = {
    let mut id = [0u8; 32];
    let bytes = b"org.arenaos.phase13app";
    let mut index = 0;
    while index < bytes.len() {
        id[index] = bytes[index];
        index += 1;
    }
    id
};

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    client::exit(99)
}

arena_desktop::entry!(main, 64 * 1024);

extern "C" fn main() -> ! {
    match arena_runtime::startup::run(|view| application_main(view)) {
        Ok(()) | Err(_) => client::exit(70),
    }
}

fn application_main(view: StartupView<'_>) -> ! {
    let actual_entry = _start as *const () as usize as u64;
    let has_document = view.capability_count() == 4;
    if view.application_id() != &APPLICATION_ID
        || view.argument_count() != 1
        || view.argument(0) != Some(b"org.arenaos.phase13app")
        || !matches!(view.capability_count(), 3 | 4)
        || view.entry() != actual_entry
        || view.load_base() != 0x0020_0000
        || view.instance_generation() == 0
    {
        client::exit(71);
    }
    let (kind, dark, motion, path) = match app_client::startup() {
        Ok(startup) => startup,
        Err(error) => client::exit(80 + error.unsigned_abs() % 20),
    };
    if kind != 6 || dark || !motion || path != [0; 32] {
        client::exit(73);
    }
    app_client::audit(kind, view.instance_generation()).unwrap_or_else(|_| client::exit(74));
    verify_vm();
    verify_heap();

    let window = Client::connect_v2(320, 180, "Phase13 Probe").unwrap_or_else(|_| client::exit(75));
    if has_document {
        let descriptor = view.capability(3).unwrap_or_else(|| client::exit(81));
        let mut actual = [0u64; 3];
        if unsafe { app_client::describe_raw(4, &mut actual) } != 0
            || descriptor.kind != 12
            || !view.capability_matches(3, actual)
        {
            client::exit(82);
        }
        let (page, base) = window.file_page();
        let files = Files::session(4, window.backing_slot(), base, page)
            .unwrap_or_else(|_| client::exit(83));
        let mut document = [0u8; 128];
        let count = files
            .read(4, 0, &mut document)
            .unwrap_or_else(|_| client::exit(84));
        if &document[..count] != b"Phase 13 associated document\n"
            || files.write(4, 0, b"must remain read-only").is_ok()
        {
            client::exit(85);
        }
        client::log(b"[phase13-app] exact read-only document capability verified\n");
    }
    let mut windows: [Option<Client>; 3] = [None, None, None];
    windows[0] = Some(window);
    let count = if has_document { 1 } else { 3 };
    if count == 3 {
        let first = windows[0].as_ref().unwrap_or_else(|| client::exit(135));
        let second = first
            .create_window(320, 180, "Phase13 Second")
            .unwrap_or_else(|_| client::exit(136));
        let third = first
            .create_window(320, 180, "Phase13 Third")
            .unwrap_or_else(|_| client::exit(137));
        windows[1] = Some(second);
        windows[2] = Some(third);
    }
    for (index, window) in windows.iter().take(count).enumerate() {
        paint(window.as_ref().unwrap_or_else(|| client::exit(138)), index);
    }
    if !has_document {
        client::log(b"[phase13-installed-app] ABI-v2 startup verified; real window published\n");
        client::log(
            b"[phase13-multiwindow] one process owns three separately backed ordinary windows\n",
        );
    }

    let mut open = [true, count == 3, count == 3];
    loop {
        app_client::idle(None).unwrap_or_else(|_| client::exit(77));
        let mut close_windows = [false; 3];
        loop {
            for index in 0..count {
                if open[index]
                    && let Some(Event::Close) = windows[index]
                        .as_ref()
                        .unwrap_or_else(|| client::exit(139))
                        .poll()
                        .unwrap_or_else(|_| client::exit(78))
                {
                    close_windows[index] = true;
                }
            }
            let more = (0..count).any(|index| {
                open[index]
                    && windows[index]
                        .as_ref()
                        .is_some_and(|window| window.more.get())
            });
            if !more {
                break;
            }
        }
        for index in 0..count {
            if open[index] && close_windows[index] {
                windows[index]
                    .as_ref()
                    .unwrap_or_else(|| client::exit(140))
                    .close_window()
                    .unwrap_or_else(|_| client::exit(141));
                open[index] = false;
                if open[..count].iter().any(|live| *live) {
                    client::log(
                        b"[phase13-window] DestroyWindow retired one surface; process and siblings remain live\n",
                    );
                } else {
                    client::log(b"[phase13-window] final surface retired; process exits cleanly\n");
                }
            }
        }
        if !open[..count].iter().any(|live| *live) {
            client::exit(42);
        }
    }
}

fn paint(window: &Client, index: usize) {
    let marker = [0x00e04020, 0x00f0d020, 0x0020d080][index.min(2)];
    for y in 0..window.height {
        for x in 0..window.width {
            // The compositor places the client raster underneath its title
            // chrome, so the unique guest-proof marker sits below that band.
            let color = if x < 24 && (32..56).contains(&y) {
                marker
            } else if (x / 32 + y / 24) & 1 == 0 {
                0x0028_6c9c
            } else {
                0x003b_9168
            };
            // SAFETY: every pixel lies inside this window's mapped surface.
            unsafe { window.pixels.add(y * window.width + x).write(color) };
        }
    }
    window.damage().unwrap_or_else(|_| client::exit(76));
}

fn verify_heap() {
    let initial = APPLICATION_HEAP
        .query()
        .unwrap_or_else(|_| client::exit(113));
    if initial != HeapUsage::default() {
        client::exit(114);
    }

    let mut bytes = Vec::<u8>::new();
    if bytes.try_reserve_exact(256 * 1024).is_err() {
        client::exit(115);
    }
    bytes.resize(256 * 1024, 0x37);
    for index in (0..bytes.len()).step_by(PAGE_BYTES) {
        if bytes[index] != 0x37 {
            client::exit(116);
        }
        bytes[index] = (index / PAGE_BYTES) as u8;
    }
    if bytes.last() != Some(&0x37) {
        client::exit(117);
    }
    let allocated = APPLICATION_HEAP
        .query()
        .unwrap_or_else(|_| client::exit(118));
    if allocated.reserved_pages != MAX_SCALABLE_HEAP_PAGES as u32
        || allocated.committed_pages != 65
        || allocated.large_pages != 65
        || allocated.small_pages != 0
    {
        client::exit(119);
    }
    drop(bytes);
    let freed = APPLICATION_HEAP
        .query()
        .unwrap_or_else(|_| client::exit(120));
    if freed.committed_pages != 65 || freed.large_pages != 0 {
        client::exit(121);
    }

    let mut reused = Vec::<u8>::new();
    if reused.try_reserve_exact(256 * 1024).is_err() {
        client::exit(122);
    }
    reused.resize(256 * 1024, 0x5A);
    if reused.first() != Some(&0x5A) || reused.last() != Some(&0x5A) {
        client::exit(123);
    }
    drop(reused);

    let small_layout = Layout::from_size_align(48, 16).unwrap_or_else(|_| client::exit(124));
    // SAFETY: the global allocator returns a 16-byte-aligned block for this
    // layout, and it is released exactly once below.
    let small = unsafe { alloc(small_layout) };
    if small.is_null() || !(small as usize).is_multiple_of(16) {
        client::exit(125);
    }
    unsafe { dealloc(small, small_layout) };

    let aligned_layout =
        Layout::from_size_align(24, PAGE_BYTES * 16).unwrap_or_else(|_| client::exit(126));
    // SAFETY: this over-aligned layout is within ScalableHeap's documented
    // 2 MiB maximum and is released exactly once below.
    let aligned = unsafe { alloc(aligned_layout) };
    if aligned.is_null() || !(aligned as usize).is_multiple_of(PAGE_BYTES * 16) {
        client::exit(127);
    }
    unsafe { dealloc(aligned, aligned_layout) };

    let mut oversized = Vec::<u8>::new();
    if oversized
        .try_reserve_exact(MAX_SCALABLE_HEAP_PAGES * PAGE_BYTES + 1)
        .is_ok()
    {
        client::exit(128);
    }
    let after_oom = APPLICATION_HEAP
        .query()
        .unwrap_or_else(|_| client::exit(129));
    if after_oom.reserved_pages != MAX_SCALABLE_HEAP_PAGES as u32 || after_oom.committed_pages < 65
    {
        client::exit(130);
    }
    client::log(b"[phase13-heap] lazy 16 MiB VM heap, 256 KiB Vec, 64-page commit batches, reuse, 64 KiB alignment, fallible OOM passed\n");
}

fn verify_vm() {
    use arena_runtime::vm::{Protection, Region};

    let region = Region::reserve(4).unwrap_or_else(|_| client::exit(90));
    let slot = region.cap_slot();
    let region_base = region.base();
    let cap = HeldCapability::from_slot(slot).unwrap_or_else(|_| client::exit(91));
    if cap.kind() != CAP_KIND_VM_REGION
        || u64::from(cap.rights()) != (RIGHTS_READ | RIGHTS_WRITE | RIGHTS_DESTROY)
        || u64::from(cap.rights()) & arena_lib::abi::RIGHTS_COPY != 0
    {
        client::exit(92);
    }
    let before = region.query().unwrap_or_else(|_| client::exit(93));
    if before.capacity_pages != 4
        || before.committed_pages != 0
        || before.guard_pages != 1
        || before.global_regions == 0
    {
        client::exit(94);
    }

    // Pointer validation must reject both guard pages and an uncommitted
    // page even though the reservation's whole span is registered.
    let probe = |address: u64| unsafe { syscall2(SYS_CAP_DESCRIBE, 1, address) };
    if probe(region.base() - 4096) != STATUS_BAD_ADDRESS
        || probe(region.base() + 4 * 4096) != STATUS_BAD_ADDRESS
        || probe(region.base() + 2 * 4096) != STATUS_BAD_ADDRESS
    {
        client::exit(95);
    }

    region
        .commit(0, 2, Protection::READ_WRITE)
        .unwrap_or_else(|_| client::exit(96));
    let committed = region.query().unwrap_or_else(|_| client::exit(97));
    if committed.committed_pages != 2
        || committed.global_committed_pages != before.global_committed_pages + 2
    {
        client::exit(98);
    }
    if region.commit(0, 1, Protection::READ_WRITE)
        != Err(arena_runtime::vm::Error::Status(STATUS_BUSY))
    {
        client::exit(99);
    }

    let words = region.base() as *mut u64;
    // SAFETY: commit returned two present RW pages and the runtime owns this
    // process-local reservation until release below.
    unsafe {
        if words.read() != 0 || words.add(512).read() != 0 {
            client::exit(100);
        }
        words.write(0xA13E_000D_51A7_0001);
        words.add(512).write(0xA13E_000D_51A7_0002);
    }

    region
        .protect(0, 1, Protection::READ_ONLY)
        .unwrap_or_else(|_| client::exit(101));
    if probe(region.base()) != STATUS_BAD_ADDRESS {
        client::exit(102);
    }
    // A W+X request must be refused before changing the existing RO PTE.
    let wx = unsafe {
        syscall6(
            SYS_VM_PROTECT,
            u64::from(slot),
            0,
            1,
            VM_PROT_READ | VM_PROT_WRITE | VM_PROT_EXEC,
            0,
            0,
        )
    };
    if wx != STATUS_BAD_ARG || probe(region.base()) != STATUS_BAD_ADDRESS {
        client::exit(103);
    }
    // Read-only and read-execute mappings remain readable while syscall
    // outputs are refused because neither mapping grants write permission.
    unsafe {
        if words.read() != 0xA13E_000D_51A7_0001 {
            client::exit(131);
        }
    }
    region
        .protect(0, 1, Protection::READ_EXECUTE)
        .unwrap_or_else(|_| client::exit(132));
    if probe(region.base()) != STATUS_BAD_ADDRESS
        || unsafe { words.read() } != 0xA13E_000D_51A7_0001
    {
        client::exit(133);
    }
    // A wrong-kind capability cannot commit into this process's VM arena.
    let wrong_kind =
        unsafe { syscall6(SYS_VM_COMMIT, 1, 0, 1, VM_PROT_READ | VM_PROT_WRITE, 0, 0) };
    if wrong_kind != STATUS_BAD_ARG {
        client::exit(104);
    }

    region
        .protect(0, 2, Protection::READ_WRITE)
        .unwrap_or_else(|_| client::exit(105));
    // SAFETY: the two VM pages are RW again after the exact-cap protection
    // transition; this verifies that contents survived permission changes.
    unsafe {
        if words.read() != 0xA13E_000D_51A7_0001 || words.add(512).read() != 0xA13E_000D_51A7_0002 {
            client::exit(106);
        }
    }

    let cap_slot = slot;
    region.release().unwrap_or_else(|_| client::exit(107));
    if probe(region_base) != STATUS_BAD_ADDRESS {
        client::exit(134);
    }
    let mut stale_query = [0u64; 6];
    if unsafe {
        syscall6(
            arena_lib::abi::SYS_VM_QUERY,
            u64::from(cap_slot),
            stale_query.as_mut_ptr() as u64,
            0,
            0,
            0,
            0,
        )
    } != STATUS_BAD_ARG
    {
        client::exit(108);
    }

    // Re-reserve and query through a new exact cap: global committed-page
    // accounting must have returned to the pre-test value after release.
    let accounting = Region::reserve(1).unwrap_or_else(|_| client::exit(109));
    let after = accounting.query().unwrap_or_else(|_| client::exit(110));
    if after.global_committed_pages != before.global_committed_pages
        || after.global_regions != before.global_regions
    {
        client::exit(111);
    }
    let accounting_slot = accounting.cap_slot();
    if unsafe { syscall1(SYS_CAP_DESTROY, u64::from(accounting_slot)) } != 0 {
        client::exit(112);
    }
    client::log(b"[phase13-vm] guarded reserve, lazy commit, RW/RO/RX protection, W^X refusal, exact release/accounting passed\n");
}
