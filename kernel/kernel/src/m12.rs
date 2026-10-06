//! Phase 12.4/12.5: native startup ABI and bounded-heap proof on the real
//! ring-3 path.
//!
//! The guest is independently linked against arena-runtime. The kernel gives
//! it a read-only slot-0 SharedRegion and one explicit Notification in slot 1.
//! The writable staging cap is destroyed before the guest runs. The valid
//! launch checks slot descriptions, argv/env, entry, RSP/RFLAGS, 32-page heap
//! OOM/reuse and teardown. Hostile controls prove refusal before app code and
//! non-overwrite of the runtime's reserved heap-cap slot.

use crate::arch::x86_64::syscall;
use crate::cap::{self, Cap, CapObj};
use crate::log::{log_error as error, log_info as info, write_marker};
use crate::{frames, ipc, proc, sched, shared, spawn, timekeeping, timer};

type Res = Result<(), &'static str>;
const PROOF_BOOT_IMAGE: u32 = 10;
const CASE_COUNT: usize = 6;
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
            mutation: Some((0, b'X')),
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
            "startup/runtime: independent guest verified ABI-v2 caps/argv/env/entry/RSP/IF/DF; FS-base TLS survived checked kernel-thread/timer handoff; bounded heap reused/coalesced blocks, filled 32 mapped pages, refused page 33 without mutation, then process teardown restored frames/maps exactly; four startup RED controls refused before app entry; heap-slot collision preserved the attenuated Notification cap and returned OOM"
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
    ];
    let child = match spawn::spawn_init_boot(PROOF_BOOT_IMAGE, &grants, None) {
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
    if cap::occupancy(child) != Some((1, cap::CAP_SLOTS as u32)) {
        return Err("startup/heap caps did not return to the single described Notification");
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
