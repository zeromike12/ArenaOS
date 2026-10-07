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
    STATUS_BAD_ARG, STATUS_BUSY, SYS_CAP_DESCRIBE, SYS_CAP_DESTROY, SYS_NOTIFY, SYS_VM_COMMIT,
    SYS_VM_PROTECT, SYS_WAIT, VM_PROT_EXEC, VM_PROT_READ, VM_PROT_WRITE, syscall1, syscall2,
    syscall6,
};
use arena_runtime::capabilities::HeldCapability;
use arena_runtime::heap::{HeapUsage, MAX_SCALABLE_HEAP_PAGES, PAGE_BYTES, ScalableHeap};
use arena_runtime::streams::{Channel, Error as StreamError, NativeStreams, STREAM_CAPACITY};
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
    let has_document = view.capability(3).is_some_and(|capability| {
        capability.role == arena_startup_abi::startup::CapabilityRole::Other
            && capability.kind == arena_startup_abi::startup::CAP_KIND_BADGED_ENDPOINT
    });
    if view.application_id() != &APPLICATION_ID
        || view.argument_count() != 1
        || view.argument(0) != Some(b"org.arenaos.phase13app")
        || view.capability_count() != if has_document { 6 } else { 5 }
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
    let streams = match NativeStreams::from_startup(&view) {
        Ok(streams) => streams,
        Err(error) => stream_startup_error(error),
    };
    if unsafe {
        syscall2(
            SYS_NOTIFY,
            streams.stream_cap_slot(),
            arena_runtime::streams::STREAM_WAKE_BADGE,
        )
    } != STATUS_BAD_ARG
    {
        client::exit(157);
    }
    let mut stdin = streams
        .set()
        .reader(Channel::Stdin)
        .unwrap_or_else(|_| client::exit(158));
    let mut stdout = streams
        .set()
        .writer(Channel::Stdout)
        .unwrap_or_else(|_| client::exit(159));
    let mut stderr = streams
        .set()
        .writer(Channel::Stderr)
        .unwrap_or_else(|_| client::exit(160));
    let mut empty = [0u8; 1];
    if stdin.read(&mut empty) != Err(StreamError::WouldBlock) {
        client::exit(161);
    }
    client::log(b"[phase13-stream] empty stdin waits through the AppInstance notification\n");
    verify_stream_output(&streams, &mut stdout, &mut stderr);
    verify_vm();
    verify_heap();

    let unknown = helper_id(b"org.arenaos.phase13unknown");
    if app_client::spawn_helper(unknown).is_ok() {
        client::exit(145);
    }
    client::log(b"[phase13-helper] unknown signed helper ID refused without spawn\n");
    let worker = app_client::spawn_helper(helper_id(b"org.arenaos.phase13worker"))
        .unwrap_or_else(|_| client::exit(142));
    if app_client::wait_helper(worker).unwrap_or_else(|_| client::exit(143)) != 43 {
        client::exit(144);
    }
    client::log(b"[phase13-helper] signed helper wait returned exact exit=43\n");

    let sleeper = app_client::spawn_helper(helper_id(b"org.arenaos.phase13sleeper"))
        .unwrap_or_else(|_| client::exit(146));
    loop {
        let badge = unsafe { syscall1(SYS_WAIT, 3) };
        if badge < 0 {
            client::exit(152);
        }
        if badge as u64 & 0x13A2 == 0x13A2 {
            break;
        }
    }
    app_client::terminate_helper(sleeper).unwrap_or_else(|_| client::exit(147));
    client::log(b"[phase13-helper] owner-authorized terminate/reap passed\n");

    let crasher = app_client::spawn_helper(helper_id(b"org.arenaos.phase13crasher"))
        .unwrap_or_else(|_| client::exit(148));
    if app_client::wait_helper(crasher).unwrap_or_else(|_| client::exit(149)) != 262 {
        client::exit(150);
    }
    client::log(b"[phase13-helper] crashed helper status=262 observed and reaped\n");

    let crasher_again = app_client::spawn_helper(helper_id(b"org.arenaos.phase13crasher"))
        .unwrap_or_else(|_| client::exit(153));
    if app_client::wait_helper(crasher_again).unwrap_or_else(|_| client::exit(154)) != 262 {
        client::exit(155);
    }
    client::log(b"[phase13-helper] retired private timer object reclaimed after reap\n");

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
    let mut typed = [0u8; 14];
    let mut typed_len = 0usize;
    let mut stdin_proved = false;
    loop {
        app_client::idle(None).unwrap_or_else(|_| client::exit(77));
        if !stdin_proved {
            loop {
                let mut bytes = [0u8; 32];
                match stdin.read(&mut bytes) {
                    Ok(0) => client::exit(162),
                    Ok(count) => {
                        for byte in &bytes[..count] {
                            if typed_len < typed.len() {
                                typed[typed_len] = *byte;
                                typed_len += 1;
                            } else {
                                typed.copy_within(1.., 0);
                                typed[typed.len() - 1] = *byte;
                            }
                            if typed_len == typed.len() && &typed == b"native-stream\r" {
                                stdin_proved = true;
                                client::log(b"[phase13-stream] stdin received exact native keyboard bytes\n");
                                break;
                            }
                        }
                        streams.wake_broker().unwrap_or_else(|_| client::exit(163));
                        if stdin_proved {
                            break;
                        }
                    }
                    Err(StreamError::WouldBlock) => break,
                    Err(_) => client::exit(164),
                }
            }
        }
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
            app_client::spawn_helper(helper_id(b"org.arenaos.phase13orphan"))
                .unwrap_or_else(|_| client::exit(151));
            client::log(b"[phase13-helper] live helper left for AppInstance owner cleanup\n");
            client::exit(42);
        }
    }
}

fn verify_stream_output(
    streams: &NativeStreams,
    stdout: &mut arena_runtime::streams::Writer<'_>,
    stderr: &mut arena_runtime::streams::Writer<'_>,
) {
    let payload = [b'Q'; STREAM_CAPACITY + 32];
    let accepted = stdout.write(&payload).unwrap_or_else(|_| client::exit(165));
    if accepted != STREAM_CAPACITY
        || stdout.write(&payload[accepted..]) != Err(StreamError::WouldBlock)
    {
        client::exit(166);
    }
    client::log(b"[phase13-stream] output full; extra write returned WouldBlock\n");
    write_stream_bytes(streams, stdout, &payload[accepted..]);
    let marker = b"\n[phase13-stream] stdout partial transfer reached the broker\n";
    write_stream_bytes(streams, stdout, marker);
    write_stream_bytes(
        streams,
        stderr,
        b"[phase13-stream] stderr channel reached the broker\n",
    );
    stdout.close();
    stderr.close();
    streams.wake_broker().unwrap_or_else(|_| client::exit(167));
}

fn stream_startup_error(error: StreamError) -> ! {
    let (message, code): (&[u8], u64) = match error {
        StreamError::BadAddress => (b"[phase13-stream] attach rejected address\n", 156),
        StreamError::TooSmall => (b"[phase13-stream] attach rejected page size\n", 157),
        StreamError::BadFormat => (b"[phase13-stream] attach rejected format\n", 158),
        StreamError::WouldBlock => (b"[phase13-stream] startup attach would block\n", 159),
        StreamError::BrokenPipe => (b"[phase13-stream] startup attach broken peer\n", 160),
        StreamError::Closed => (b"[phase13-stream] startup attach closed peer\n", 161),
        StreamError::Corrupt => (b"[phase13-stream] startup attach found corrupt ring\n", 162),
        StreamError::EndpointInUse => (b"[phase13-stream] duplicate standard stream attach\n", 163),
        StreamError::MissingAuthority => (
            b"[phase13-stream] exact stream cap authority mismatch\n",
            164,
        ),
        StreamError::CapabilityMismatch => (
            b"[phase13-stream] held cap differs from startup descriptor\n",
            165,
        ),
        StreamError::RegionGeometry(status) => {
            client::log(b"[phase13-stream] stream page query returned status=");
            log_signed(status);
            client::log(b"\n");
            (b"[phase13-stream] stream region is not one page\n", 166)
        }
        StreamError::Kernel(_) => (
            b"[phase13-stream] kernel refused standard stream map\n",
            167,
        ),
    };
    client::log(message);
    client::exit(code)
}

fn log_signed(value: i64) {
    if value < 0 {
        client::log(b"-");
    }
    let mut number = value.unsigned_abs();
    let mut digits = [0u8; 20];
    let mut length = 0usize;
    loop {
        digits[length] = b'0' + (number % 10) as u8;
        length += 1;
        number /= 10;
        if number == 0 {
            break;
        }
    }
    for index in (0..length).rev() {
        client::log(&digits[index..=index]);
    }
}

fn write_stream_bytes(
    streams: &NativeStreams,
    writer: &mut arena_runtime::streams::Writer<'_>,
    bytes: &[u8],
) {
    let mut offset = 0usize;
    while offset < bytes.len() {
        match writer.write(&bytes[offset..]) {
            Ok(0) => client::exit(168),
            Ok(count) => offset += count,
            Err(StreamError::WouldBlock) => {
                streams.wake_broker().unwrap_or_else(|_| client::exit(169));
                app_client::idle(None).unwrap_or_else(|_| client::exit(170));
            }
            Err(_) => client::exit(171),
        }
    }
    streams.wake_broker().unwrap_or_else(|_| client::exit(172));
}

fn helper_id(name: &[u8]) -> [u8; 32] {
    let mut id = [0u8; 32];
    if name.len() < id.len() {
        id[..name.len()].copy_from_slice(name);
    }
    id
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
