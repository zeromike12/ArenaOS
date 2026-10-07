#![no_std]
#![no_main]

use arena_desktop::client;
use arena_lib::abi::{
    RIGHTS_READ, RIGHTS_WRITE, STATUS_BAD_ARG, SYS_CAP_DESCRIBE, SYS_CAP_OCCUPIED,
    SYS_NOTIFICATION_CREATE, SYS_NOTIFY, SYS_THREAD_EXIT, SYS_TIMER_ARM, SYS_WAIT, syscall1,
    syscall2, syscall3, syscall6,
};
use arena_runtime::streams::{Channel, Error as StreamError, NativeStreams};
use arena_runtime::threads;
use arena_startup_abi::manifest::{FLAG_HEADLESS, FLAG_STANDARD_STREAMS};
use arena_startup_abi::startup::{
    CAP_KIND_NOTIFICATION, CAP_KIND_SHARED_REGION, CapabilityRole, StartupView,
};
use core::sync::atomic::{AtomicU64, Ordering};

const APPLICATION_ID: [u8; 32] = {
    let mut id = [0; 32];
    let bytes = b"org.arenaos.zzheadless";
    let mut index = 0;
    while index < bytes.len() {
        id[index] = bytes[index];
        index += 1;
    }
    id
};
const INSTALLED_APP_ID: [u8; 32] = {
    let mut id = [0u8; 32];
    let bytes = b"org.arenaos.phase13app";
    let mut index = 0;
    while index < bytes.len() {
        id[index] = bytes[index];
        index += 1;
    }
    id
};
const SLEEPER_ID: &[u8] = b"org.arenaos.phase13sleeper";
const CRASHER_ID: &[u8] = b"org.arenaos.phase13crasher";
const ORPHAN_ID: &[u8] = b"org.arenaos.phase13orphan";
const STREAMER_ID: &[u8] = b"org.arenaos.phase13streamer";
const COMPLETION_BADGE: u64 = 0x13A1;
const HELPER_READY_BADGE: u64 = 0x13A2;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    client::exit(99)
}

arena_desktop::entry!(main, 16 * 1024);

extern "C" fn main() -> ! {
    match arena_runtime::startup::run(application_main) {
        Ok(()) | Err(_) => client::exit(70),
    }
}

fn application_main(view: StartupView<'_>) {
    if view.argument_count() == 2 {
        helper_main(view);
    }
    let mut observed = [0u64; 3];
    if view.application_id() != &APPLICATION_ID
        || view.argument_count() != 1
        || view.argument(0) != Some(b"org.arenaos.zzheadless")
        || view.flags() & FLAG_HEADLESS == 0
        || view.instance_generation() == 0
        || view.capability_count() != 1
        || view.stdin_descriptor().is_some()
        || view.stdout_descriptor().is_some()
        || view.stderr_descriptor().is_some()
        || view.capability(0).is_none_or(|descriptor| {
            descriptor.slot != 1
                || descriptor.kind != CAP_KIND_NOTIFICATION
                || u64::from(descriptor.rights) != (RIGHTS_READ | RIGHTS_WRITE)
        })
        || unsafe { syscall2(SYS_CAP_DESCRIBE, 1, observed.as_mut_ptr() as u64) } != 0
        || observed[0] != u64::from(CAP_KIND_NOTIFICATION)
        || observed[2] != (RIGHTS_READ | RIGHTS_WRITE)
    {
        client::exit(71);
    }

    client::log(b"[phase13-headless] Startup ABI v2 verified one attenuated Notification; no window caps present\n");
    let timer = unsafe { syscall3(SYS_TIMER_ARM, 1, COMPLETION_BADGE, 3_000_000) };
    if timer < 0 || unsafe { syscall1(SYS_WAIT, 1) } != COMPLETION_BADGE as i64 {
        client::exit(72);
    }
    client::log(b"[phase13-headless] timer completed; process exiting for manager reap\n");
    client::exit(42)
}

fn helper_main(view: StartupView<'_>) -> ! {
    let mut observed = [0u64; 3];
    let mut owner_signal = [0u64; 3];
    let streaming = view.flags() & FLAG_STANDARD_STREAMS != 0;
    let signal_parent = view.argument(1) == Some(SLEEPER_ID) || streaming;
    if view.application_id() != &INSTALLED_APP_ID
        || view.argument(0) != Some(b"org.arenaos.phase13app")
        || view.flags()
            != if streaming {
                1 | FLAG_STANDARD_STREAMS
            } else {
                1
            }
        || view.instance_generation() == 0
        || view.capability_count() != 1 + usize::from(signal_parent) + usize::from(streaming)
        || (view.stdin_descriptor().is_some() != streaming)
        || (view.stdout_descriptor().is_some() != streaming)
        || (view.stderr_descriptor().is_some() != streaming)
        || view.capability(0).is_none_or(|descriptor| {
            descriptor.slot != 1
                || descriptor.kind != CAP_KIND_NOTIFICATION
                || u64::from(descriptor.rights) != (RIGHTS_READ | RIGHTS_WRITE)
        })
        || unsafe { syscall2(SYS_CAP_DESCRIBE, 1, observed.as_mut_ptr() as u64) } != 0
        || observed[0] != u64::from(CAP_KIND_NOTIFICATION)
        || observed[2] != (RIGHTS_READ | RIGHTS_WRITE)
        || (signal_parent
            && (view.capability(1).is_none_or(|descriptor| {
                descriptor.slot != 2
                    || descriptor.kind != CAP_KIND_NOTIFICATION
                    || u64::from(descriptor.rights) != RIGHTS_WRITE
                    || descriptor.role
                        != if streaming {
                            CapabilityRole::StreamWake
                        } else {
                            CapabilityRole::Other
                        }
            }) || unsafe { syscall2(SYS_CAP_DESCRIBE, 2, owner_signal.as_mut_ptr() as u64) }
                != 0
                || owner_signal[0] != u64::from(CAP_KIND_NOTIFICATION)
                || owner_signal[1] == observed[1]
                || owner_signal[2] != RIGHTS_WRITE))
        || (streaming
            && view.capability(2).is_none_or(|descriptor| {
                descriptor.slot != 3
                    || descriptor.kind != CAP_KIND_SHARED_REGION
                    || descriptor.rights != (RIGHTS_READ | RIGHTS_WRITE) as u32
                    || descriptor.role != CapabilityRole::StandardStreamSet
            }))
    {
        client::exit(73);
    }
    if unsafe { syscall6(SYS_NOTIFICATION_CREATE, 44, 10, 0, 0, 0, 0) } != STATUS_BAD_ARG
        || unsafe { syscall6(SYS_CAP_OCCUPIED, 10, 0, 0, 0, 0, 0) } != 0
        || (signal_parent && unsafe { syscall1(SYS_WAIT, 2) } != STATUS_BAD_ARG)
        || (streaming && unsafe { syscall2(SYS_NOTIFY, 3, HELPER_READY_BADGE) } != STATUS_BAD_ARG)
    {
        client::exit(78);
    }
    client::log(b"[phase13-helper] no inherited notification factory; WRITE-only owner signal cannot wait\n");
    client::log(b"[phase13-helper] exact Startup ABI inventory: private timer Notification; optional owner signal is WRITE-only and a distinct object\n");
    if view.argument(1) == Some(SLEEPER_ID) {
        let started = AtomicU64::new(0);
        let worker = threads::spawn(live_helper_worker, &started as *const AtomicU64 as u64)
            .unwrap_or_else(|_| client::exit(87));
        for _ in 0..100_000 {
            if started.load(Ordering::Acquire) == 1 {
                break;
            }
            threads::yield_now().unwrap_or_else(|_| client::exit(88));
        }
        if started.load(Ordering::Acquire) != 1 || threads::count() != Ok(2) {
            client::exit(89);
        }
        // Keep the thread live and its exact stack cap owned by this helper
        // until the Desktop destroys the Process. Its worker has run in ring
        // 3 and remains in the scheduler's Ready set at the readiness signal.
        core::mem::forget(worker);
        client::log(b"[phase13-headless] sleeper ran a live ring-3 worker before parent-authorized teardown\n");
    }
    match view.argument(1) {
        Some(id) if id == SLEEPER_ID => wait_for_helper_timer(44, true),
        Some(id) if id == ORPHAN_ID => wait_for_helper_timer(45, false),
        Some(id) if id == CRASHER_ID => {
            client::log(
                b"[phase13-helper] intentional helper fault for Process-cap crash/reap proof\n",
            );
            // The kernel isolates this ordinary user #UD as the helper's
            // stable Process-cap status 0x100 + vector 6.
            unsafe { core::arch::asm!("ud2", options(noreturn, nostack)) }
        }
        Some(id) if id == STREAMER_ID => stream_helper(view),
        _ => client::exit(74),
    }
}

extern "C" fn live_helper_worker(started: u64) -> u64 {
    // SAFETY: the helper's main thread owns this AtomicU64 on its mapped
    // process stack and remains alive until the Desktop tears down the whole
    // Process, including this user-created worker.
    let started = unsafe { &*(started as *const AtomicU64) };
    started.store(1, Ordering::Release);
    loop {
        if threads::yield_now().is_err() {
            core::hint::spin_loop();
        }
    }
}

fn stream_helper(view: StartupView<'_>) -> ! {
    let streams = NativeStreams::from_startup(&view).unwrap_or_else(|_| client::exit(79));
    let mut stdin = streams
        .set()
        .reader(Channel::Stdin)
        .unwrap_or_else(|_| client::exit(80));
    let mut stdout = streams
        .set()
        .writer(Channel::Stdout)
        .unwrap_or_else(|_| client::exit(81));
    let mut input = [0u8; 32];
    loop {
        match stdin.read(&mut input) {
            Ok(count) if &input[..count] == b"helper-input" => break,
            Ok(_) => client::exit(82),
            Err(StreamError::WouldBlock) => {
                if unsafe { syscall1(SYS_WAIT, 1) } < 0 {
                    client::exit(83);
                }
            }
            Err(_) => client::exit(84),
        }
    }
    if stdout.write(b"helper-output") != Ok(13) {
        client::exit(85);
    }
    stdout.close();
    stdin.close();
    streams.wake_broker().unwrap_or_else(|_| client::exit(86));
    client::log(b"[phase13-helper-stream] child transferred stdin and stdout bytes over its exact stream set\n");
    helper_exit(46)
}

fn wait_for_helper_timer(exit_status: u64, signal_parent: bool) -> ! {
    let timer = unsafe { syscall3(SYS_TIMER_ARM, 1, COMPLETION_BADGE, 5_000_000) };
    if timer < 0 {
        client::exit(75);
    }
    if signal_parent {
        if unsafe { syscall2(SYS_NOTIFY, 2, HELPER_READY_BADGE) } != 0 {
            client::exit(77);
        }
        client::log(
            b"[phase13-helper] readiness badge sent through separate WRITE-only owner signal\n",
        );
    }
    loop {
        let badge = unsafe { syscall1(SYS_WAIT, 1) };
        if badge < 0 {
            client::exit(76);
        }
        if badge as u64 & COMPLETION_BADGE == COMPLETION_BADGE {
            helper_exit(exit_status);
        }
    }
}

fn helper_exit(status: u64) -> ! {
    let _ = unsafe { syscall1(SYS_THREAD_EXIT, status) };
    loop {
        core::hint::spin_loop();
    }
}
