#![no_std]
#![no_main]

extern crate alloc;

use alloc::{
    alloc::{alloc, dealloc},
    boxed::Box,
    vec::Vec,
};
use arena_desktop::{
    app_client,
    client::{self, Client},
    files::Files,
    model::Event as WindowEvent,
};
use arena_lib::abi::{
    CAP_KIND_VM_REGION, RIGHTS_DESTROY, RIGHTS_READ, RIGHTS_WRITE, STATUS_BAD_ADDRESS,
    STATUS_BAD_ARG, STATUS_BUSY, STATUS_QUOTA, SYS_CAP_DESCRIBE, SYS_CAP_DESTROY, SYS_NOTIFY,
    SYS_SYNC_KEY_CREATE, SYS_SYNC_KEY_DESTROY, SYS_SYNC_SEQUENCE, SYS_SYNC_WAIT, SYS_SYNC_WAKE,
    SYS_THREAD_CREATE, SYS_THREAD_DETACH, SYS_THREAD_JOIN, SYS_VM_COMMIT, SYS_VM_PROTECT,
    SYS_VM_RELEASE, SYS_WAIT, VM_PROT_EXEC, VM_PROT_READ, VM_PROT_WRITE, syscall1, syscall2,
    syscall6,
};
use arena_runtime::capabilities::HeldCapability;
use arena_runtime::heap::{HeapUsage, MAX_SCALABLE_HEAP_PAGES, PAGE_BYTES, ScalableHeap};
use arena_runtime::streams::{Channel, Error as StreamError, NativeStreams, STREAM_CAPACITY};
use arena_runtime::sync::{Condvar, Event as SyncEvent, Mutex, Once, SyncDomain};
use arena_runtime::threads::{self, Error as ThreadError, JoinHandle};
use arena_runtime::tls;
use arena_runtime::vm::{Protection, Region};
use arena_startup_abi::startup::StartupView;
use core::alloc::Layout;
use core::sync::atomic::{AtomicU64, Ordering};

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
        || view.flags()
            != (arena_startup_abi::manifest::FLAG_MULTI_INSTANCE
                | arena_startup_abi::manifest::FLAG_STANDARD_STREAMS
                | arena_startup_abi::manifest::FLAG_NATIVE_SYNC)
        || view.capability_count() != if has_document { 7 } else { 6 }
        || view.entry() != actual_entry
        || view.load_base() != 0x0020_0000
        || view.instance_generation() == 0
    {
        client::exit(71);
    }
    let sync_domain = SyncDomain::from_startup(&view).unwrap_or_else(|_| client::exit(178));
    let sync_slot = view
        .capability_for_role(arena_startup_abi::startup::CapabilityRole::SyncDomain)
        .unwrap_or_else(|| client::exit(178))
        .slot;
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
    verify_threads();
    verify_sync(sync_domain, u64::from(sync_slot));

    let unknown = helper_id(b"org.arenaos.phase13unknown");
    if app_client::spawn_helper(unknown).is_ok() {
        client::exit(145);
    }
    client::log(b"[phase13-helper] unknown signed helper ID refused without spawn\n");
    if app_client::spawn_helper(helper_id(b"org.arenaos.phase13streamer")).is_ok()
        || app_client::spawn_helper_streams(helper_id(b"org.arenaos.phase13sleeper")).is_ok()
    {
        client::exit(174);
    }
    client::log(
        b"[phase13-helper-stream] signed stream policy and explicit launch request must match\n",
    );
    let (stream_helper, helper_streams) =
        app_client::spawn_helper_streams(helper_id(b"org.arenaos.phase13streamer"))
            .unwrap_or_else(|_| client::exit(142));
    let mut helper_stdin = helper_streams
        .set()
        .writer(Channel::Stdin)
        .unwrap_or_else(|_| client::exit(143));
    let mut helper_stdout = helper_streams
        .set()
        .reader(Channel::Stdout)
        .unwrap_or_else(|_| client::exit(144));
    if unsafe {
        syscall2(
            SYS_NOTIFY,
            helper_streams.stream_cap_slot(),
            arena_runtime::streams::STREAM_WAKE_BADGE,
        )
    } != STATUS_BAD_ARG
    {
        client::exit(175);
    }
    if app_client::wake_helper(stream_helper.wrapping_add(1)).is_ok() {
        client::exit(176);
    }
    if helper_stdin.write(b"helper-input") != Ok(12) {
        client::exit(165);
    }
    app_client::wake_helper(stream_helper).unwrap_or_else(|_| client::exit(166));
    let mut helper_output = [0u8; 13];
    let mut helper_output_len = 0;
    while helper_output_len < helper_output.len() {
        match helper_stdout.read(&mut helper_output[helper_output_len..]) {
            Ok(0) => client::exit(167),
            Ok(count) => helper_output_len += count,
            Err(StreamError::WouldBlock) => {
                app_client::idle(None).unwrap_or_else(|_| client::exit(168));
            }
            Err(_) => client::exit(169),
        }
    }
    if &helper_output != b"helper-output" {
        client::exit(170);
    }
    helper_stdin.close();
    drop(helper_stdin);
    if app_client::wait_helper(stream_helper).unwrap_or_else(|_| client::exit(171)) != 46 {
        client::exit(172);
    }
    let mut eof = [0u8; 1];
    if helper_stdout.read(&mut eof) != Ok(0) || !helper_stdout.writer_closed() {
        client::exit(173);
    }
    drop(helper_stdout);
    drop(helper_streams);
    client::log(b"[phase13-helper-stream] owner woke child; exact stdin/stdout bytes transferred; EOF after reap\n");

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
    let sync_after_helper = sync_domain.info().unwrap_or_else(|_| client::exit(214));
    if sync_after_helper.keys != 0 || sync_after_helper.parked_waiters != 0 {
        client::exit(215);
    }
    client::log(b"[phase13-sync] helper process death reclaimed its key and parked waiter\n");
    client::log(b"[phase13-helper] owner-authorized terminate/reap passed\n");

    let (crasher, crasher_streams) =
        app_client::spawn_helper_streams(helper_id(b"org.arenaos.phase13crasher"))
            .unwrap_or_else(|_| client::exit(148));
    let mut crasher_stdout = crasher_streams
        .set()
        .reader(Channel::Stdout)
        .unwrap_or_else(|_| client::exit(174));
    if app_client::wait_helper(crasher).unwrap_or_else(|_| client::exit(149)) != 262 {
        client::exit(150);
    }
    if crasher_stdout.read(&mut eof) != Ok(0) || !crasher_stdout.writer_closed() {
        client::exit(175);
    }
    drop(crasher_stdout);
    drop(crasher_streams);
    client::log(b"[phase13-helper] crashed helper status=262 observed and reaped\n");
    client::log(b"[phase13-helper-stream] crashed child's output reached EOF after exact Process-cap reap\n");

    let (crasher_again, crasher_again_streams) =
        app_client::spawn_helper_streams(helper_id(b"org.arenaos.phase13crasher"))
            .unwrap_or_else(|_| client::exit(153));
    let mut crasher_again_stdout = crasher_again_streams
        .set()
        .reader(Channel::Stdout)
        .unwrap_or_else(|_| client::exit(176));
    if app_client::wait_helper(crasher_again).unwrap_or_else(|_| client::exit(154)) != 262 {
        client::exit(155);
    }
    if crasher_again_stdout.read(&mut eof) != Ok(0) || !crasher_again_stdout.writer_closed() {
        client::exit(177);
    }
    drop(crasher_again_stdout);
    drop(crasher_again_streams);
    client::log(b"[phase13-helper-stream] crashed child's output reached EOF after exact Process-cap reap\n");
    client::log(b"[phase13-helper] retired private timer object reclaimed after reap\n");

    app_client::spawn_helper(helper_id(b"org.arenaos.phase13orphan"))
        .unwrap_or_else(|_| client::exit(151));
    client::log(b"[phase13-helper] live helper left for AppInstance owner cleanup\n");
    let mut pressure_threads = if has_document {
        Some(start_pressure_threads(sync_domain))
    } else {
        None
    };
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
                    && let Some(WindowEvent::Close) = windows[index]
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
            if let Some(threads) = pressure_threads.take() {
                finish_pressure_threads(threads);
            }
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

struct PressureThreadFixture {
    mutex: Mutex<u64>,
    condition: Condvar,
    ready: AtomicU64,
    failures: AtomicU64,
}

#[derive(Clone, Copy)]
struct PressureThreadArgument {
    fixture: *const PressureThreadFixture,
    ordinal: u64,
}

struct PressureThreads {
    domain: SyncDomain,
    fixture: Box<PressureThreadFixture>,
    _arguments: [PressureThreadArgument; 2],
    handles: [Option<JoinHandle>; 2],
}

extern "C" fn pressure_thread(argument: u64) -> u64 {
    // SAFETY: the AppInstance keeps its fixture and argument array alive
    // until the final ordinary window closes and both threads are joined.
    let argument = unsafe { (argument as *const PressureThreadArgument).read_volatile() };
    let fixture = unsafe { &*argument.fixture };
    let mut guard = fixture.mutex.lock().unwrap_or_else(|_| client::exit(178));
    fixture.ready.fetch_add(1, Ordering::Release);
    while *guard == 0 {
        guard = fixture
            .condition
            .wait(guard)
            .unwrap_or_else(|_| client::exit(179));
    }
    argument.ordinal
}

fn start_pressure_threads(domain: SyncDomain) -> PressureThreads {
    let fixture = Box::new(PressureThreadFixture {
        mutex: Mutex::new(domain, 0).unwrap_or_else(|_| client::exit(180)),
        condition: Condvar::new(domain).unwrap_or_else(|_| client::exit(181)),
        ready: AtomicU64::new(0),
        failures: AtomicU64::new(0),
    });
    let fixture_ptr = &*fixture as *const PressureThreadFixture;
    let arguments = [
        PressureThreadArgument {
            fixture: fixture_ptr,
            ordinal: 0,
        },
        PressureThreadArgument {
            fixture: fixture_ptr,
            ordinal: 1,
        },
    ];
    let mut handles = [None, None];
    for index in 0..2 {
        handles[index] = Some(
            threads::spawn(
                pressure_thread,
                &arguments[index] as *const PressureThreadArgument as u64,
            )
            .unwrap_or_else(|_| client::exit(182)),
        );
    }
    let mut parked = false;
    for _ in 0..100_000 {
        if fixture.ready.load(Ordering::Acquire) == 2
            && domain.info().is_ok_and(|info| info.parked_waiters == 2)
        {
            parked = true;
            break;
        }
        threads::yield_now().unwrap_or_else(|_| client::exit(183));
    }
    if !parked {
        client::exit(184);
    }
    client::log(
        b"[phase13-threads] two ring-3 workers remain parked on the live document AppInstance\n",
    );
    PressureThreads {
        domain,
        fixture,
        _arguments: arguments,
        handles,
    }
}

fn finish_pressure_threads(mut active: PressureThreads) {
    {
        let mut guard = active
            .fixture
            .mutex
            .lock()
            .unwrap_or_else(|_| client::exit(185));
        *guard = 1;
        if active.fixture.condition.notify_all() != Ok(2) {
            client::exit(186);
        }
    }
    for (ordinal, handle) in active.handles.iter_mut().enumerate() {
        let result = handle
            .take()
            .unwrap_or_else(|| client::exit(187))
            .join()
            .unwrap_or_else(|_| client::exit(188));
        if result.exit_status != ordinal as u64 {
            client::exit(189);
        }
    }
    if active.fixture.failures.load(Ordering::Relaxed) != 0 {
        client::exit(190);
    }
    drop(active.fixture);
    let clean = active.domain.info().unwrap_or_else(|_| client::exit(191));
    if clean.keys != 0 || clean.parked_waiters != 0 || threads::count() != Ok(1) {
        client::exit(192);
    }
    client::log(
        b"[phase13-threads] closing the document window woke and joined both live workers\n",
    );
}

struct ThreadFixture {
    ready: AtomicU64,
    release: AtomicU64,
    heap_updates: AtomicU64,
    failures: AtomicU64,
    readonly_address: u64,
}

#[derive(Clone, Copy)]
struct ThreadArgument {
    fixture: *const ThreadFixture,
    ordinal: u64,
}

extern "C" fn thread_worker(argument: u64) -> u64 {
    // SAFETY: verify_threads keeps both the shared heap object and argument
    // array alive until every created thread has been joined or detached.
    let argument = unsafe { (argument as *const ThreadArgument).read_volatile() };
    let fixture = unsafe { &*argument.fixture };
    let Some(tcb) = tls::current() else {
        fixture.failures.fetch_add(1, Ordering::Relaxed);
        return 0x1300_0000 + argument.ordinal;
    };
    // Each thread writes only its own FS:8 TCB word. The parent reads it
    // after join, when the kernel has published this thread's exit status.
    unsafe {
        core::ptr::addr_of_mut!((*tcb.as_ptr()).application_word)
            .write_volatile(0xA13E_0000 + argument.ordinal);
    }
    fixture.ready.fetch_add(1, Ordering::Release);
    while fixture.release.load(Ordering::Acquire) == 0 {
        if threads::yield_now().is_err() {
            fixture.failures.fetch_add(1, Ordering::Relaxed);
            return 0x1300_1000 + argument.ordinal;
        }
    }
    let readonly = unsafe { (fixture.readonly_address as *const u64).read_volatile() };
    if readonly != 0xA13E_1300_0000_000D {
        fixture.failures.fetch_add(1, Ordering::Relaxed);
    }
    for _ in 0..64 {
        fixture.heap_updates.fetch_add(1, Ordering::AcqRel);
        if threads::yield_now().is_err() {
            fixture.failures.fetch_add(1, Ordering::Relaxed);
            return 0x1300_2000 + argument.ordinal;
        }
    }
    0xA13E_0000 + argument.ordinal
}

fn verify_threads() {
    let readonly = Region::reserve(1).unwrap_or_else(|_| client::exit(178));
    readonly
        .commit(0, 1, Protection::READ_WRITE)
        .unwrap_or_else(|_| client::exit(179));
    let readonly_value = 0xA13E_1300_0000_000D;
    unsafe { (readonly.base() as *mut u64).write_volatile(readonly_value) };
    readonly
        .protect(0, 1, Protection::READ_ONLY)
        .unwrap_or_else(|_| client::exit(180));

    let fixture = alloc::boxed::Box::new(ThreadFixture {
        ready: AtomicU64::new(0),
        release: AtomicU64::new(0),
        heap_updates: AtomicU64::new(0),
        failures: AtomicU64::new(0),
        readonly_address: readonly.base(),
    });
    let fixture_ptr = &*fixture as *const ThreadFixture;
    let mut arguments = [ThreadArgument {
        fixture: fixture_ptr,
        ordinal: 0,
    }; 5];
    for (ordinal, argument) in arguments.iter_mut().enumerate() {
        argument.ordinal = ordinal as u64;
    }

    let vm_before = readonly.query().unwrap_or_else(|_| client::exit(181));
    let mut handles: [Option<JoinHandle>; 4] = [None, None, None, None];
    for index in 0..4 {
        let arg = &arguments[index] as *const ThreadArgument as u64;
        let handle = threads::spawn(thread_worker, arg).unwrap_or_else(|error| match error {
            ThreadError::Status(STATUS_QUOTA) => client::exit(182),
            _ => client::exit(183),
        });
        if handles[..index]
            .iter()
            .flatten()
            .any(|existing| existing.tls_base() == handle.tls_base())
        {
            client::exit(184);
        }
        handles[index] = Some(handle);
    }
    let live_vm = readonly.query().unwrap_or_else(|_| client::exit(185));
    if live_vm.global_regions != vm_before.global_regions + 4
        || live_vm.global_committed_pages != vm_before.global_committed_pages + 60
    {
        client::exit(186);
    }
    let quota = threads::spawn(thread_worker, &arguments[4] as *const ThreadArgument as u64);
    if !matches!(quota, Err(ThreadError::Status(STATUS_QUOTA))) {
        client::exit(187);
    }
    let mut ready = false;
    for _ in 0..100_000 {
        if fixture.ready.load(Ordering::Acquire) == 4 {
            ready = true;
            break;
        }
        threads::yield_now().unwrap_or_else(|_| client::exit(188));
    }
    if !ready || threads::count() != Ok(5) {
        client::exit(189);
    }

    let first = handles[0].as_ref().unwrap_or_else(|| client::exit(190));
    let first_slot = first.stack_cap_slot().unwrap_or_else(|| client::exit(191));
    let first_base = first.tls_base();
    let stack_low = first_base + 2 * PAGE_BYTES as u64;
    let stack_top = first_base + 16 * PAGE_BYTES as u64;
    if unsafe { syscall6(SYS_VM_RELEASE, u64::from(first_slot), 0, 0, 0, 0, 0) } != STATUS_BUSY {
        client::exit(192);
    }
    let invalid_entry = unsafe {
        syscall6(
            SYS_THREAD_CREATE,
            first_base,
            0,
            u64::from(first_slot),
            stack_low,
            stack_top,
            first_base,
        )
    };
    if invalid_entry != STATUS_BAD_ADDRESS {
        client::exit(193);
    }
    let second = handles[1].as_ref().unwrap_or_else(|| client::exit(194));
    let second_slot = second.stack_cap_slot().unwrap_or_else(|| client::exit(195));
    if first_slot == second_slot {
        client::exit(196);
    }
    let crossed_stack = unsafe {
        syscall6(
            SYS_THREAD_CREATE,
            thread_worker as *const () as u64,
            &arguments[4] as *const ThreadArgument as u64,
            u64::from(second_slot),
            stack_low,
            stack_top,
            first_base,
        )
    };
    if crossed_stack != STATUS_BAD_ADDRESS {
        client::exit(197);
    }
    let mut bogus_status = 0u64;
    if unsafe {
        syscall6(
            SYS_THREAD_JOIN,
            u64::MAX,
            &mut bogus_status as *mut u64 as u64,
            0,
            0,
            0,
            0,
        )
    } != STATUS_BAD_ARG
        || unsafe { syscall6(SYS_THREAD_DETACH, u64::MAX, 0, 0, 0, 0, 0) } != STATUS_BAD_ARG
    {
        client::exit(198);
    }

    fixture.release.store(1, Ordering::Release);
    for (index, handle_slot) in handles.iter_mut().enumerate() {
        let handle = handle_slot.take().unwrap_or_else(|| client::exit(199));
        let id = handle.id();
        let expected = 0xA13E_0000 + index as u64;
        let outcome = handle.join().unwrap_or_else(|_| client::exit(200));
        if outcome.exit_status != expected || outcome.application_word != expected {
            client::exit(201);
        }
        if unsafe {
            syscall6(
                SYS_THREAD_JOIN,
                id,
                &mut bogus_status as *mut u64 as u64,
                0,
                0,
                0,
                0,
            )
        } != STATUS_BAD_ARG
        {
            client::exit(202);
        }
    }
    if threads::count() != Ok(1)
        || fixture.heap_updates.load(Ordering::Acquire) != 4 * 64
        || fixture.failures.load(Ordering::Relaxed) != 0
    {
        client::exit(203);
    }
    threads::yield_now().unwrap_or_else(|_| client::exit(204));
    let joined_vm = readonly.query().unwrap_or_else(|_| client::exit(205));
    if joined_vm.global_regions != vm_before.global_regions
        || joined_vm.global_committed_pages != vm_before.global_committed_pages
    {
        client::exit(206);
    }

    fixture.ready.store(0, Ordering::Release);
    fixture.release.store(0, Ordering::Release);
    let detached = threads::spawn(thread_worker, &arguments[4] as *const ThreadArgument as u64)
        .unwrap_or_else(|_| client::exit(207));
    for _ in 0..100_000 {
        if fixture.ready.load(Ordering::Acquire) == 1 {
            break;
        }
        threads::yield_now().unwrap_or_else(|_| client::exit(208));
    }
    if fixture.ready.load(Ordering::Acquire) != 1 {
        client::exit(209);
    }
    drop(detached);
    fixture.release.store(1, Ordering::Release);
    let mut detached_clean = false;
    for _ in 0..100_000 {
        threads::yield_now().unwrap_or_else(|_| client::exit(210));
        let current = readonly.query().unwrap_or_else(|_| client::exit(211));
        if threads::count() == Ok(1)
            && current.global_regions == vm_before.global_regions
            && current.global_committed_pages == vm_before.global_committed_pages
        {
            detached_clean = true;
            break;
        }
    }
    if !detached_clean || fixture.failures.load(Ordering::Relaxed) != 0 {
        client::exit(212);
    }
    readonly.release().unwrap_or_else(|_| client::exit(213));
    drop(fixture);
    client::log(b"[phase13-threads] four concurrent ring-3 threads shared heap and read-only VM; distinct FS.base TLS, quota, exact stack caps, join/detach, and cleanup passed\n");
}

struct SyncMutexFixture {
    mutex: Mutex<u64>,
    entered: AtomicU64,
    failures: AtomicU64,
}

#[derive(Clone, Copy)]
struct SyncArgument {
    fixture: *const SyncMutexFixture,
    ordinal: u64,
}

extern "C" fn mutex_worker(argument: u64) -> u64 {
    // SAFETY: verify_sync keeps the fixture and its argument array alive
    // until every worker has joined.
    let argument = unsafe { (argument as *const SyncArgument).read_volatile() };
    let fixture = unsafe { &*argument.fixture };
    fixture.entered.fetch_add(1, Ordering::Release);
    for _ in 0..24 {
        let mut guard = match fixture.mutex.lock() {
            Ok(guard) => guard,
            Err(_) => {
                fixture.failures.fetch_add(1, Ordering::Relaxed);
                return 0x1310_0000 + argument.ordinal;
            }
        };
        *guard += 1;
        if threads::yield_now().is_err() {
            fixture.failures.fetch_add(1, Ordering::Relaxed);
            return 0x1310_1000 + argument.ordinal;
        }
    }
    argument.ordinal
}

struct OnceFixture {
    once: Once,
    entered: AtomicU64,
    completed: AtomicU64,
    initialized: AtomicU64,
    failures: AtomicU64,
}

#[derive(Clone, Copy)]
struct OnceArgument {
    fixture: *const OnceFixture,
    ordinal: u64,
}

extern "C" fn once_worker(argument: u64) -> u64 {
    // SAFETY: verify_sync joins every worker before dropping the fixture.
    let argument = unsafe { (argument as *const OnceArgument).read_volatile() };
    let fixture = unsafe { &*argument.fixture };
    fixture.entered.fetch_add(1, Ordering::Release);
    if fixture
        .once
        .call_once(|| {
            fixture.initialized.fetch_add(1, Ordering::AcqRel);
            for _ in 0..16 {
                if threads::yield_now().is_err() {
                    fixture.failures.fetch_add(1, Ordering::Relaxed);
                    break;
                }
            }
        })
        .is_err()
    {
        fixture.failures.fetch_add(1, Ordering::Relaxed);
        return 0x1320_0000 + argument.ordinal;
    }
    fixture.completed.fetch_add(1, Ordering::Release);
    argument.ordinal
}

struct CondvarFixture {
    mutex: Mutex<u64>,
    condition: Condvar,
    ready: AtomicU64,
    completed: AtomicU64,
    failures: AtomicU64,
}

#[derive(Clone, Copy)]
struct CondvarArgument {
    fixture: *const CondvarFixture,
    ordinal: u64,
}

extern "C" fn condvar_worker(argument: u64) -> u64 {
    // SAFETY: verify_sync joins every worker before dropping the fixture.
    let argument = unsafe { (argument as *const CondvarArgument).read_volatile() };
    let fixture = unsafe { &*argument.fixture };
    let mut guard = match fixture.mutex.lock() {
        Ok(guard) => guard,
        Err(_) => {
            fixture.failures.fetch_add(1, Ordering::Relaxed);
            return 0x1330_0000 + argument.ordinal;
        }
    };
    fixture.ready.fetch_add(1, Ordering::Release);
    while *guard == 0 {
        guard = match fixture.condition.wait(guard) {
            Ok(guard) => guard,
            Err(_) => {
                fixture.failures.fetch_add(1, Ordering::Relaxed);
                return 0x1330_1000 + argument.ordinal;
            }
        };
    }
    fixture.completed.fetch_add(1, Ordering::Release);
    argument.ordinal
}

fn verify_sync(domain: SyncDomain, domain_slot: u64) {
    let initial = domain.info().unwrap_or_else(|_| client::exit(216));
    if initial.keys != 0 || initial.parked_waiters != 0 {
        client::exit(217);
    }
    if unsafe { syscall6(SYS_SYNC_KEY_CREATE, 1, 0, 0, 0, 0, 0) } != STATUS_BAD_ARG
        || unsafe { syscall6(SYS_SYNC_KEY_DESTROY, domain_slot, u64::MAX, 0, 0, 0, 0) }
            != STATUS_BAD_ARG
        || unsafe { syscall6(SYS_SYNC_SEQUENCE, domain_slot, u64::MAX, 0, 0, 0, 0) }
            != STATUS_BAD_ARG
        || unsafe { syscall6(SYS_SYNC_WAIT, domain_slot, u64::MAX, 1, 0, 0, 0) } != STATUS_BAD_ARG
        || unsafe { syscall6(SYS_SYNC_WAKE, domain_slot, u64::MAX, 1, 0, 0, 0) } != STATUS_BAD_ARG
    {
        client::exit(218);
    }

    {
        // A notification that races ahead of the wait is detected through
        // the captured sequence rather than being lost as a transient badge.
        let event = SyncEvent::new(domain).unwrap_or_else(|_| client::exit(219));
        let observed = event.sequence().unwrap_or_else(|_| client::exit(220));
        if event.signal_one() != Ok(0) || event.wait(observed, 0).is_err() {
            client::exit(221);
        }
    }

    {
        let fixture = SyncMutexFixture {
            mutex: Mutex::new(domain, 0).unwrap_or_else(|_| client::exit(222)),
            entered: AtomicU64::new(0),
            failures: AtomicU64::new(0),
        };
        let fixture_ptr = &fixture as *const SyncMutexFixture;
        let mut arguments = [SyncArgument {
            fixture: fixture_ptr,
            ordinal: 0,
        }; 4];
        let mut handles: [Option<JoinHandle>; 4] = [None, None, None, None];
        let held = fixture.mutex.lock().unwrap_or_else(|_| client::exit(223));
        for index in 0..4 {
            arguments[index].ordinal = index as u64;
            arguments[index].fixture = fixture_ptr;
            handles[index] = Some(
                threads::spawn(
                    mutex_worker,
                    &arguments[index] as *const SyncArgument as u64,
                )
                .unwrap_or_else(|_| client::exit(224)),
            );
        }
        let mut all_waiting = false;
        for _ in 0..100_000 {
            if fixture.entered.load(Ordering::Acquire) == 4
                && domain.info().is_ok_and(|info| info.parked_waiters == 4)
            {
                all_waiting = true;
                break;
            }
            threads::yield_now().unwrap_or_else(|_| client::exit(225));
        }
        if !all_waiting {
            client::exit(226);
        }
        drop(held);
        for (index, handle) in handles.iter_mut().enumerate() {
            let result = handle
                .take()
                .unwrap_or_else(|| client::exit(227))
                .join()
                .unwrap_or_else(|_| client::exit(228));
            if result.exit_status != index as u64 {
                client::exit(229);
            }
        }
        if *fixture.mutex.lock().unwrap_or_else(|_| client::exit(230)) != 4 * 24
            || fixture.failures.load(Ordering::Relaxed) != 0
            || !domain.info().is_ok_and(|info| info.parked_waiters == 0)
        {
            client::exit(231);
        }
    }

    {
        let fixture = OnceFixture {
            once: Once::new(domain).unwrap_or_else(|_| client::exit(232)),
            entered: AtomicU64::new(0),
            completed: AtomicU64::new(0),
            initialized: AtomicU64::new(0),
            failures: AtomicU64::new(0),
        };
        let fixture_ptr = &fixture as *const OnceFixture;
        let mut arguments = [OnceArgument {
            fixture: fixture_ptr,
            ordinal: 0,
        }; 4];
        let mut handles: [Option<JoinHandle>; 4] = [None, None, None, None];
        for index in 0..4 {
            arguments[index].ordinal = index as u64;
            arguments[index].fixture = fixture_ptr;
            handles[index] = Some(
                threads::spawn(once_worker, &arguments[index] as *const OnceArgument as u64)
                    .unwrap_or_else(|_| client::exit(233)),
            );
        }
        for _ in 0..100_000 {
            if fixture.completed.load(Ordering::Acquire) == 4 {
                break;
            }
            threads::yield_now().unwrap_or_else(|_| client::exit(234));
        }
        if fixture.entered.load(Ordering::Acquire) != 4
            || fixture.completed.load(Ordering::Acquire) != 4
            || fixture.initialized.load(Ordering::Acquire) != 1
            || fixture.failures.load(Ordering::Relaxed) != 0
            || !fixture.once.is_complete()
        {
            client::exit(235);
        }
        for (index, handle) in handles.into_iter().enumerate() {
            if handle
                .unwrap_or_else(|| client::exit(236))
                .join()
                .unwrap_or_else(|_| client::exit(237))
                .exit_status
                != index as u64
            {
                client::exit(238);
            }
        }
    }

    {
        let fixture = CondvarFixture {
            mutex: Mutex::new(domain, 0).unwrap_or_else(|_| client::exit(239)),
            condition: Condvar::new(domain).unwrap_or_else(|_| client::exit(240)),
            ready: AtomicU64::new(0),
            completed: AtomicU64::new(0),
            failures: AtomicU64::new(0),
        };
        let fixture_ptr = &fixture as *const CondvarFixture;
        let mut arguments = [CondvarArgument {
            fixture: fixture_ptr,
            ordinal: 0,
        }; 4];
        let mut handles: [Option<JoinHandle>; 4] = [None, None, None, None];
        for index in 0..4 {
            arguments[index].ordinal = index as u64;
            arguments[index].fixture = fixture_ptr;
            handles[index] = Some(
                threads::spawn(
                    condvar_worker,
                    &arguments[index] as *const CondvarArgument as u64,
                )
                .unwrap_or_else(|_| client::exit(241)),
            );
        }
        let mut all_waiting = false;
        for _ in 0..100_000 {
            if fixture.ready.load(Ordering::Acquire) == 4
                && domain.info().is_ok_and(|info| info.parked_waiters == 4)
            {
                all_waiting = true;
                break;
            }
            threads::yield_now().unwrap_or_else(|_| client::exit(242));
        }
        if !all_waiting {
            client::exit(243);
        }
        {
            let mut guard = fixture.mutex.lock().unwrap_or_else(|_| client::exit(244));
            *guard = 1;
            if fixture.condition.notify_one() != Ok(1) {
                client::exit(245);
            }
        }
        let mut one_woke = false;
        for _ in 0..100_000 {
            if fixture.completed.load(Ordering::Acquire) == 1
                && domain.info().is_ok_and(|info| info.parked_waiters == 3)
            {
                one_woke = true;
                break;
            }
            threads::yield_now().unwrap_or_else(|_| client::exit(246));
        }
        if !one_woke {
            client::exit(247);
        }
        {
            let mut guard = fixture.mutex.lock().unwrap_or_else(|_| client::exit(248));
            *guard = 2;
            if fixture.condition.notify_all() != Ok(3) {
                client::exit(249);
            }
        }
        for (index, handle) in handles.into_iter().enumerate() {
            if handle
                .unwrap_or_else(|| client::exit(250))
                .join()
                .unwrap_or_else(|_| client::exit(251))
                .exit_status
                != index as u64
            {
                client::exit(252);
            }
        }
        if fixture.completed.load(Ordering::Acquire) != 4
            || fixture.failures.load(Ordering::Relaxed) != 0
            || !domain.info().is_ok_and(|info| info.parked_waiters == 0)
        {
            client::exit(253);
        }
    }

    {
        let mutex = Mutex::new(domain, ()).unwrap_or_else(|_| client::exit(254));
        let condition = Condvar::new(domain).unwrap_or_else(|_| client::exit(255));
        let guard = mutex.lock().unwrap_or_else(|_| client::exit(256));
        let (guard, timed_out) = condition
            .wait_timeout(guard, 100_000)
            .unwrap_or_else(|_| client::exit(257));
        if !timed_out {
            client::exit(258);
        }
        drop(guard);
    }
    let clean = domain.info().unwrap_or_else(|_| client::exit(259));
    if clean.keys != 0 || clean.parked_waiters != 0 || clean.keys_high < 2 {
        client::exit(260);
    }
    client::log(b"[phase13-sync] contended Mutex, multi-waiter Condvar wake-one/all, sequence-before-wait, Once contention, timeout, invalid-cap refusal, and key accounting passed\n");
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
