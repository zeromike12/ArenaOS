//! Phase 12.4/12.5: native startup ABI and bounded-heap proof on the real
//! ring-3 path.
//!
//! The guest is independently linked against arena-runtime. The kernel gives
//! it a read-only slot-0 SharedRegion and one explicit Notification in slot 1.
//! The writable staging cap is destroyed before the guest runs. The valid
//! launch checks slot descriptions, argv/env, entry, RSP/RFLAGS, 32-page heap
//! OOM/reuse and teardown. Hostile controls prove refusal before app code,
//! exact cap-table inventory, and non-overwrite of the reserved heap-cap slot.

use crate::arch::x86_64::syscall;
use crate::cap::{self, Cap, CapObj};
use crate::log::{log_error as error, log_info as info, write_marker};
use crate::{frames, ipc, proc, sched, shared, spawn, timekeeping, timer};
use core::sync::atomic::{AtomicUsize, Ordering};

type Res = Result<(), &'static str>;
const PROOF_BOOT_IMAGE: u32 = 10;
const CASE_COUNT: usize = 7;
const IPC_QUEUE_DEPTH_PROOF: usize = 32;
const IPC_QUEUE_REQUEST: [u64; 2] = [0x12A0_0001, 0x12A0_0002];
static IPC_QUEUE_EID: AtomicUsize = AtomicUsize::new(0);
static IPC_QUEUE_ACCEPTED: AtomicUsize = AtomicUsize::new(0);
static IPC_QUEUE_REFUSED: AtomicUsize = AtomicUsize::new(0);
static IPC_QUEUE_OTHER: AtomicUsize = AtomicUsize::new(0);
const STARTUP_REFUSED: u64 = 0xA12;
const APP_EXIT_OK: u64 = 37;
const STACK_TOP_EXPECTED: u64 = 0x0021_4000;
const APP_MESSAGE: &[u8] = b"m12:startup:application: PASS\n";
const REFUSAL_MESSAGE: &[u8] = b"arena-runtime: startup refused\n";

const VALID_PAGE: &[u8; 4096] =
    include_bytes!("../../../userspace/arena-runtime/tests/data/startup-runtime-v2.bin");
const WRONG_RIGHTS_PAGE: &[u8; 4096] =
    include_bytes!("../../../userspace/arena-runtime/tests/data/startup-runtime-bad-rights-v2.bin");
const HEAP_SLOT_PAGE: &[u8; 4096] =
    include_bytes!("../../../userspace/arena-runtime/tests/data/startup-runtime-heap-slot-v2.bin");

#[derive(Clone, Copy)]
struct Case {
    name: &'static str,
    page: &'static [u8; 4096],
    pages: u32,
    startup_rights: u32,
    notification_rights: u32,
    extra_cap: Option<Cap>,
    mutation: Option<(usize, u8)>,
    expected_status: u64,
    expected_write: &'static [u8],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Resources {
    free_frames: u64,
    processes: usize,
    notifications: usize,
    shared: (usize, u32, usize),
}

fn resources() -> Resources {
    Resources {
        free_frames: frames::free_frames(),
        processes: proc::live_count(),
        notifications: ipc::notification_occupancy(),
        shared: shared::usage_snapshot(),
    }
}

pub fn run_suite() -> bool {
    let image = match spawn::boot_image_bytes(PROOF_BOOT_IMAGE) {
        Some(image) => image,
        None => {
            error!("m12", "startup proof image is absent");
            write_marker(format_args!("m12: RESULT FAIL (0/{CASE_COUNT})"));
            return false;
        }
    };
    let parsed = match crate::elf::validate(image) {
        Ok(info) => info,
        Err(_) => {
            error!(
                "m12",
                "startup proof image failed the production ELF parser"
            );
            write_marker(format_args!("m12: RESULT FAIL (0/{CASE_COUNT})"));
            return false;
        }
    };
    let base = parsed.segs[..parsed.nsegs]
        .iter()
        .map(|segment| segment.vaddr)
        .min()
        .unwrap_or(0);
    let image_top = parsed.segs[..parsed.nsegs]
        .iter()
        .map(|segment| segment.vaddr.saturating_add(segment.memsz))
        .max()
        .unwrap_or(0);
    let stack_top = image_top.div_ceil(4096) * 4096 + 4096;
    if parsed.entry != 0x0020_0000 || base != 0x0020_0000 || stack_top != STACK_TOP_EXPECTED {
        error!(
            "m12",
            "startup proof link facts differ: entry={:#x} base={:#x} RSP-top={:#x}",
            parsed.entry,
            base,
            stack_top
        );
        write_marker(format_args!("m12: RESULT FAIL (0/{CASE_COUNT})"));
        return false;
    }

    let cases = [
        Case {
            name: "runtime_guest",
            page: VALID_PAGE,
            pages: 1,
            startup_rights: cap::RIGHTS_READ | cap::RIGHTS_DESTROY,
            notification_rights: cap::RIGHTS_READ | cap::RIGHTS_WRITE,
            extra_cap: None,
            mutation: None,
            expected_status: APP_EXIT_OK,
            expected_write: APP_MESSAGE,
        },
        Case {
            name: "listed_cap_rights_red",
            page: WRONG_RIGHTS_PAGE,
            pages: 1,
            startup_rights: cap::RIGHTS_READ | cap::RIGHTS_DESTROY,
            notification_rights: cap::RIGHTS_READ | cap::RIGHTS_WRITE,
            extra_cap: None,
            mutation: None,
            expected_status: STARTUP_REFUSED,
            expected_write: REFUSAL_MESSAGE,
        },
        Case {
            name: "startup_cap_rights_red",
            page: VALID_PAGE,
            pages: 1,
            startup_rights: cap::RIGHTS_READ | cap::RIGHTS_WRITE | cap::RIGHTS_DESTROY,
            notification_rights: cap::RIGHTS_READ | cap::RIGHTS_WRITE,
            extra_cap: None,
            mutation: None,
            expected_status: STARTUP_REFUSED,
            expected_write: REFUSAL_MESSAGE,
        },
        Case {
            name: "startup_page_count_red",
            page: VALID_PAGE,
            pages: 2,
            startup_rights: cap::RIGHTS_READ | cap::RIGHTS_DESTROY,
            notification_rights: cap::RIGHTS_READ | cap::RIGHTS_WRITE,
            extra_cap: None,
            mutation: None,
            expected_status: STARTUP_REFUSED,
            expected_write: REFUSAL_MESSAGE,
        },
        Case {
            name: "malformed_record_red",
            page: VALID_PAGE,
            pages: 1,
            startup_rights: cap::RIGHTS_READ | cap::RIGHTS_DESTROY,
            notification_rights: cap::RIGHTS_READ | cap::RIGHTS_WRITE,
            extra_cap: None,
            mutation: Some((0, b'X')),
            expected_status: STARTUP_REFUSED,
            expected_write: REFUSAL_MESSAGE,
        },
        Case {
            name: "unlisted_capability_red",
            page: VALID_PAGE,
            pages: 1,
            startup_rights: cap::RIGHTS_READ | cap::RIGHTS_DESTROY,
            notification_rights: cap::RIGHTS_READ | cap::RIGHTS_WRITE,
            extra_cap: Some(Cap {
                obj: CapObj::MemoryPool,
                rights: cap::RIGHTS_WRITE,
            }),
            mutation: None,
            expected_status: STARTUP_REFUSED,
            expected_write: REFUSAL_MESSAGE,
        },
        Case {
            name: "heap_slot_collision_red",
            page: HEAP_SLOT_PAGE,
            pages: 1,
            startup_rights: cap::RIGHTS_READ | cap::RIGHTS_DESTROY,
            notification_rights: cap::RIGHTS_READ
                | cap::RIGHTS_WRITE
                | cap::RIGHTS_COPY
                | cap::RIGHTS_DESTROY,
            extra_cap: Some(Cap {
                obj: CapObj::BootImage {
                    index: PROOF_BOOT_IMAGE,
                },
                rights: cap::RIGHTS_READ,
            }),
            mutation: None,
            expected_status: APP_EXIT_OK,
            expected_write: APP_MESSAGE,
        },
    ];

    let baseline = resources();
    let mut passed = 0usize;
    for case in cases {
        let before = resources();
        match run_case(case) {
            Ok(()) if resources() == before => {
                passed += 1;
                write_marker(format_args!("m12:test:{}: PASS", case.name));
            }
            Ok(()) => {
                error!(
                    "m12",
                    "{} left resources behind: {:?} -> {:?}",
                    case.name,
                    before,
                    resources()
                );
                write_marker(format_args!(
                    "m12:test:{}: FAIL (resource delta)",
                    case.name
                ));
            }
            Err(reason) => {
                error!("m12", "{} failed: {}", case.name, reason);
                write_marker(format_args!("m12:test:{}: FAIL ({})", case.name, reason));
            }
        }
    }

    let all_released = resources() == baseline;
    if passed == cases.len() && all_released {
        info!(
            "m12",
            "startup/runtime: independent guest verified ABI-v2 caps/argv/env/entry/RSP/IF/DF; FS-base TLS survived checked kernel-thread/timer handoff; bounded heap reused/coalesced blocks, filled 32 mapped pages, refused page 33 without mutation, then process teardown restored frames/maps exactly; five startup RED controls refused before app entry, including an unlisted live cap; heap-slot collision preserved the attenuated Notification cap and returned OOM"
        );
        write_marker(format_args!("m12: RESULT PASS ({passed}/{})", cases.len()));
        true
    } else {
        error!(
            "m12",
            "startup ABI suite {passed}/{} passed; final resources {:?} baseline {:?}",
            cases.len(),
            resources(),
            baseline
        );
        write_marker(format_args!("m12: RESULT FAIL ({passed}/{})", cases.len()));
        false
    }
}

/// Phase-12 endpoint-queue regression: a 32-client burst must occupy the
/// configured queue, the 33rd caller must receive mutation-free STATUS_BUSY,
/// and all 32 accepted requests must still be served exactly once.
pub fn run_ipc_queue_capacity_test() -> bool {
    let result = ipc_queue_capacity_test();
    match result {
        Ok(()) => {
            info!(
                "m12",
                "IPC queue accepted exactly 32 blocked callers, refused caller 33 with STATUS_BUSY, served all accepted requests once, and restored all threads/endpoints"
            );
            write_marker(format_args!("m12:ipc_queue32: PASS"));
            true
        }
        Err(reason) => {
            error!("m12", "IPC queue capacity proof failed: {reason}");
            write_marker(format_args!("m12:ipc_queue32: FAIL ({reason})"));
            false
        }
    }
}

fn ipc_queue_client(_: usize) {
    let eid = IPC_QUEUE_EID.load(Ordering::Relaxed) as u32;
    match ipc::call(0, eid, IPC_QUEUE_REQUEST, None, [0; ipc::MSG_BYTES]) {
        Ok((words, landed, message))
            if words == [IPC_QUEUE_REQUEST[0] + 1, IPC_QUEUE_REQUEST[1]]
                && landed == ipc::CAP_NONE
                && message == [0; ipc::MSG_BYTES] =>
        {
            IPC_QUEUE_ACCEPTED.fetch_add(1, Ordering::Relaxed);
        }
        Err(syscall::STATUS_BUSY) => {
            IPC_QUEUE_REFUSED.fetch_add(1, Ordering::Relaxed);
        }
        _ => {
            IPC_QUEUE_OTHER.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn ipc_queue_capacity_test() -> Res {
    let base_threads = sched::live_threads();
    let eid = ipc::create_endpoint().map_err(|_| "could not allocate proof endpoint")?;
    IPC_QUEUE_EID.store(eid as usize, Ordering::Relaxed);
    IPC_QUEUE_ACCEPTED.store(0, Ordering::Relaxed);
    IPC_QUEUE_REFUSED.store(0, Ordering::Relaxed);
    IPC_QUEUE_OTHER.store(0, Ordering::Relaxed);

    for _ in 0..IPC_QUEUE_DEPTH_PROOF {
        let tid = sched::spawn("m12-ipc-queue-client", ipc_queue_client, 0)
            .map_err(|_| "could not spawn one of 32 queue clients")?;
        let mut blocked = false;
        for _ in 0..64 {
            if sched::thread_blocked(tid) {
                blocked = true;
                break;
            }
            sched::yield_now();
        }
        if !blocked {
            return Err("one of the first 32 callers was not queued and blocked");
        }
    }

    let overflow = sched::spawn("m12-ipc-queue-overflow", ipc_queue_client, 0)
        .map_err(|_| "could not spawn the 33rd queue client")?;
    for _ in 0..64 {
        if IPC_QUEUE_REFUSED.load(Ordering::Relaxed) != 0 || sched::thread_blocked(overflow) {
            break;
        }
        sched::yield_now();
    }
    if IPC_QUEUE_REFUSED.load(Ordering::Relaxed) != 1
        || IPC_QUEUE_ACCEPTED.load(Ordering::Relaxed) != 0
        || IPC_QUEUE_OTHER.load(Ordering::Relaxed) != 0
        || sched::thread_blocked(overflow)
    {
        return Err(
            "caller 33 was not refused exactly once without disturbing the 32 queued calls",
        );
    }
    if sched::live_threads() != base_threads + IPC_QUEUE_DEPTH_PROOF {
        return Err("queue saturation did not retain exactly the 32 blocked caller threads");
    }

    for expected in 1..=IPC_QUEUE_DEPTH_PROOF {
        let (words, landed, message) =
            ipc::recv(0, eid).map_err(|_| "a previously queued call could not be received")?;
        if words != IPC_QUEUE_REQUEST || landed != ipc::CAP_NONE || message != [0; ipc::MSG_BYTES] {
            return Err("a queued request changed while caller 33 was refused");
        }
        ipc::reply(
            eid,
            [IPC_QUEUE_REQUEST[0] + 1, IPC_QUEUE_REQUEST[1]],
            None,
            [0; ipc::MSG_BYTES],
        )
        .map_err(|_| "could not reply to an accepted queue caller")?;
        let mut completed = false;
        for _ in 0..64 {
            sched::yield_now();
            if IPC_QUEUE_ACCEPTED.load(Ordering::Relaxed) == expected {
                completed = true;
                break;
            }
        }
        if !completed {
            return Err("an accepted caller did not receive its exact reply");
        }
    }

    if IPC_QUEUE_ACCEPTED.load(Ordering::Relaxed) != IPC_QUEUE_DEPTH_PROOF
        || IPC_QUEUE_REFUSED.load(Ordering::Relaxed) != 1
        || IPC_QUEUE_OTHER.load(Ordering::Relaxed) != 0
        || sched::live_threads() != base_threads
    {
        return Err("queue clients or thread slots did not return to their exact baseline");
    }
    ipc::destroy_endpoint(eid).map_err(|_| "proof endpoint did not retire cleanly")?;
    if ipc::endpoint_live(eid) {
        return Err("proof endpoint remained live after teardown");
    }
    Ok(())
}

fn run_case(case: Case) -> Res {
    let owner = proc::create("m12-startup-owner")?;
    let (source_slot, region_id) = shared::create(owner, case.pages)
        .ok_or("could not allocate the bounded startup SharedRegion")?;
    let (physical, actual_pages) =
        shared::backing(region_id).ok_or("startup SharedRegion backing disappeared")?;
    if actual_pages != case.pages {
        return Err("startup SharedRegion page count differs from request");
    }
    // SAFETY: the new region is a zeroed, allocator-owned physical run; this
    // kernel test is its sole writer and initializes the page before spawn.
    unsafe {
        core::ptr::copy_nonoverlapping(
            case.page.as_ptr(),
            (physical + crate::arch::x86_64::paging::KERNEL_OFFSET) as *mut u8,
            4096,
        );
        if let Some((offset, value)) = case.mutation {
            core::ptr::write_volatile(
                (physical + crate::arch::x86_64::paging::KERNEL_OFFSET + offset as u64) as *mut u8,
                value,
            );
        }
    }

    let notification = ipc::create_notification()?;
    let grants = [
        Cap {
            obj: CapObj::SharedRegion { id: region_id },
            rights: case.startup_rights,
        },
        Cap {
            obj: CapObj::Notification { nid: notification },
            rights: case.notification_rights,
        },
        case.extra_cap.unwrap_or(Cap::EMPTY),
    ];
    let grant_count = if case.extra_cap.is_some() { 3 } else { 2 };
    let child = match spawn::spawn_init_boot(PROOF_BOOT_IMAGE, &grants[..grant_count], None) {
        Ok(pid) => pid,
        Err(_) => {
            let _ = ipc::destroy_notification(notification);
            let _ = cap::destroy(owner, source_slot);
            let _ = proc::destroy(owner);
            return Err("independent startup guest spawn refused");
        }
    };

    // Revoke the manager's writable source before yielding to the child. The
    // guest can observe only the exact READ|DESTROY cap in slot 0.
    cap::destroy(owner, source_slot).map_err(|_| "startup source cap cleanup refused")?;
    proc::destroy(owner).map_err(|_| "startup owner process cleanup refused")?;

    let records = spawn::records_snapshot();
    let tid = records
        .iter()
        .flatten()
        .find_map(|&(pid, tid)| (pid == child).then_some(tid))
        .ok_or("startup guest has no exact spawn record")?;
    let status = wait_for_exit(tid)?;
    if status != case.expected_status {
        error!(
            "m12",
            "{}: guest exit {status}, expected {}", case.name, case.expected_status
        );
        return Err("startup guest exited with the wrong status");
    }
    let (last, last_len) = syscall::last_write();
    if last_len != case.expected_write.len() || &last[..last_len] != case.expected_write {
        return Err("startup guest's last copied diagnostic does not match expectation");
    }

    if timer::held_by(child) != 0 {
        return Err("TLS timer was not retired before guest teardown");
    }
    let expected_occupied = if case.extra_cap.is_some() { 2 } else { 1 };
    if cap::occupancy(child) != Some((expected_occupied, cap::CAP_SLOTS as u32)) {
        return Err("startup capabilities did not return to their exact case baseline");
    }
    proc::destroy(child).map_err(|_| "startup guest process teardown refused")?;
    spawn::forget(child).map_err(|_| "startup guest spawn record retirement refused")?;
    ipc::destroy_notification(notification)
        .map_err(|_| "startup guest Notification teardown refused")?;
    Ok(())
}

fn wait_for_exit(tid: u64) -> Result<u64, &'static str> {
    let interrupts_were_enabled = crate::arch::x86_64::interrupts_enabled();
    crate::arch::x86_64::sti();
    let result = (|| {
        let deadline = timekeeping::now_us().saturating_add(5_000_000);
        loop {
            if let Some(status) = syscall::exit_status_of(tid) {
                sched::yield_now();
                return Ok(status);
            }
            if timekeeping::now_us() >= deadline {
                return Err("independent startup guest exceeded its bounded deadline");
            }
            sched::yield_now();
        }
    })();
    if !interrupts_were_enabled {
        crate::arch::x86_64::cli();
    }
    result
}
