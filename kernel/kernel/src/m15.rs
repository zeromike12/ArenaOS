//! Phase-15 native FP/SIMD guest qualification (proposed ADR-0112).
//!
//! A handwritten ET_EXEC fixture runs at CPL3. It checks the clean FXSAVE64
//! template on first entry, x87 and scalar/SSE2 arithmetic, rounding modes,
//! all sixteen XMM registers, cooperative yield, blocking join, timer
//! preemption, cross-CR3 switches, slot reuse, and contained #MF/#XF/#UD.

use crate::arch::x86_64::{self, paging, syscall};
use crate::elf;
use crate::frames;
use crate::log::{log_error as error, log_info as info, write_marker};
use crate::proc;
use crate::sched::{self, UserThreadStart};
use crate::sync::without_interrupts;

const USER_STACK_BASE: u64 = 0x0050_0000;
const USER_PAGE: u64 = paging::PAGE;
const SPIN_TIMEOUT_US: u64 = 10_000_000;
const ARG_JOIN: u64 = 1 << 63;
const ARG_FAULT: u64 = 1 << 62;
const EXIT_OK: u64 = 42;

#[derive(Clone, Copy)]
struct ProcessFixture {
    pid: u64,
    entry: u64,
    stack_pages: usize,
}

pub fn run_suite() -> bool {
    let checks: [(&str, fn() -> Result<(), &'static str>); 3] = [
        (
            "thread_isolation_block_yield_preempt",
            test_thread_isolation,
        ),
        (
            "process_isolation_slot_reuse",
            test_process_isolation_and_reuse,
        ),
        ("fault_containment", test_fault_containment),
    ];
    let mut passed = 0u32;
    for (name, test) in checks {
        match test() {
            Ok(()) => {
                passed += 1;
                write_marker(format_args!("m15:test:{name}: PASS"));
            }
            Err(reason) => {
                error!("m15", "test {name} failed: {reason}");
                write_marker(format_args!("m15:test:{name}: FAIL ({reason})"));
            }
        }
    }
    if passed == checks.len() as u32 {
        info!(
            "m15",
            "native x87/SSE state tests passed ({passed}/{})",
            checks.len()
        );
        write_marker(format_args!("m15: RESULT PASS ({passed}/{})", checks.len()));
        true
    } else {
        write_marker(format_args!("m15: RESULT FAIL ({passed}/{})", checks.len()));
        false
    }
}

fn prepare_process(name: &'static str, stack_pages: usize) -> Result<ProcessFixture, &'static str> {
    if stack_pages == 0 || stack_pages > 4 {
        return Err("invalid Phase-15 stack-page count");
    }
    let pid = proc::create(name)?;
    let loaded = elf::load(elf::FPU_TEST_IMAGE, pid)?;
    let root = proc::pml4_of(pid).ok_or("Phase-15 process lost its PML4")?;
    let stack_end = USER_STACK_BASE + stack_pages as u64 * USER_PAGE;
    without_interrupts(|| {
        for page in 0..stack_pages {
            let Some(frame) = frames::alloc() else {
                return Err("no frame for Phase-15 user stack");
            };
            let va = USER_STACK_BASE + page as u64 * USER_PAGE;
            // SAFETY: `frame` is fresh and exclusively owned; the direct-map
            // alias is writable, and the new process owns this user leaf.
            let mapped = unsafe {
                core::ptr::write_bytes(
                    frame.wrapping_add(paging::KERNEL_OFFSET) as *mut u8,
                    0,
                    USER_PAGE as usize,
                );
                paging::map_user_page_4k(root, va, frame, true, false)
            };
            if mapped.is_err() {
                let _ = frames::free(frame);
                return Err("could not map Phase-15 user stack page");
            }
        }
        proc::append_user_region(pid, USER_STACK_BASE, stack_end)
    })?;
    Ok(ProcessFixture {
        pid,
        entry: loaded.entry,
        stack_pages,
    })
}

fn spawn_fixture_thread(
    process: ProcessFixture,
    stack_index: usize,
    argument: u64,
) -> Result<u64, &'static str> {
    if stack_index >= process.stack_pages {
        return Err("Phase-15 thread stack index is out of range");
    }
    let low = USER_STACK_BASE + stack_index as u64 * USER_PAGE;
    let top = low + USER_PAGE;
    sched::spawn_user_in_proc(
        process.pid,
        UserThreadStart {
            entry: process.entry,
            argument,
            user_rsp: top - 8,
            fs_base: 0,
            stack_low: low,
            stack_top: top,
            // The kernel-owned fixture maps guarded test stacks directly;
            // no userspace capability is exposed or needed by this test.
            stack_region_id: 0xF000 + stack_index as u32,
            stack_cap_slot: u8::MAX,
        },
    )
}

fn drive_tasks(tasks: &[(u64, u64)]) -> Result<u64, &'static str> {
    let saved_flags = x86_64::read_flags();
    let preempt_before = sched::preempt::total();
    x86_64::cli();
    sched::preempt::enable(1);
    // Every entered user context has IF=1; enabling here also exercises the
    // same timer path if the bootstrap thread is selected between workers.
    unsafe { x86_64::restore_flags(saved_flags | x86_64::RFLAGS_IF) };
    let started = crate::timekeeping::now_us();
    let mut complete = false;
    while crate::timekeeping::now_us().wrapping_sub(started) < SPIN_TIMEOUT_US {
        complete = tasks
            .iter()
            .all(|(id, _expected)| syscall::exit_status_of(*id).is_some());
        if complete {
            break;
        }
        sched::yield_now();
    }
    x86_64::cli();
    sched::preempt::disable();
    // Restore the bootstrap thread's original interrupt posture exactly.
    unsafe { x86_64::restore_flags(saved_flags) };
    if !complete {
        for (id, expected) in tasks {
            error!(
                "m15",
                "task {id} timed out waiting for {expected:#x}; observed status {:?}",
                syscall::exit_status_of(*id)
            );
        }
        return Err("native FP/SIMD guest tasks exceeded the 10-second bound");
    }
    let switches = sched::preempt::total().saturating_sub(preempt_before);
    if switches == 0 {
        return Err("FP/SIMD fixture passed without any timer-driven context switch");
    }
    Ok(switches)
}

fn check_exit_statuses(tasks: &[(u64, u64)]) -> Result<(), &'static str> {
    if tasks
        .iter()
        .all(|(id, expected)| syscall::exit_status_of(*id) == Some(*expected))
    {
        Ok(())
    } else {
        Err("one or more native FP/SIMD guest tasks returned an unexpected status")
    }
}

fn destroy_process(process: ProcessFixture) -> Result<(), &'static str> {
    proc::destroy(process.pid)?;
    // The process teardown marks its exited user records joined; the next
    // scheduler pass retires kernel stacks and scrubs their FXSAVE slots.
    sched::yield_now();
    Ok(())
}

fn test_thread_isolation() -> Result<(), &'static str> {
    let free_before = frames::free_frames();
    let threads_before = sched::live_threads();
    let process = prepare_process("m15Threads", 3)?;
    let worker_a = spawn_fixture_thread(process, 0, 1)?;
    let worker_b = spawn_fixture_thread(process, 1, 2)?;
    let joiner = spawn_fixture_thread(process, 2, ARG_JOIN | worker_b)?;
    let slot_a = sched::slot_of_thread(worker_a).ok_or("worker A has no scheduler slot")?;
    let switches = drive_tasks(&[(worker_a, EXIT_OK), (worker_b, EXIT_OK), (joiner, EXIT_OK)])?;
    check_exit_statuses(&[(worker_a, EXIT_OK), (worker_b, EXIT_OK), (joiner, EXIT_OK)])?;
    if syscall::exit_status_of(worker_b) != Some(EXIT_OK) {
        return Err("joined worker B did not return its independent status");
    }
    destroy_process(process)?;
    if sched::slot_of_thread(worker_a).is_some() {
        return Err("reaped user thread retained its scheduler slot");
    }
    if sched::live_threads() != threads_before {
        return Err("same-process user threads did not drain to baseline");
    }
    if frames::free_frames() != free_before {
        return Err("same-process image, stacks, or kernel thread frames leaked");
    }
    info!(
        "m15",
        "thread isolation: two distinct x87/MXCSR/XMM0-XMM15 workers plus a blocking joiner; yield/syscall/timer preemption ({switches} switches), slot {slot_a} scrubbed and resources exact"
    );
    Ok(())
}

fn test_process_isolation_and_reuse() -> Result<(), &'static str> {
    let free_before = frames::free_frames();
    let threads_before = sched::live_threads();
    let process_a = prepare_process("m15ProcA", 1)?;
    let process_b = prepare_process("m15ProcB", 1)?;
    let thread_a = spawn_fixture_thread(process_a, 0, 1)?;
    let thread_b = spawn_fixture_thread(process_b, 0, 2)?;
    let reused_slot = sched::slot_of_thread(thread_a).ok_or("process A task has no slot")?;
    let switches = drive_tasks(&[(thread_a, EXIT_OK), (thread_b, EXIT_OK)])?;
    check_exit_statuses(&[(thread_a, EXIT_OK), (thread_b, EXIT_OK)])?;
    destroy_process(process_a)?;
    destroy_process(process_b)?;
    if sched::slot_of_thread(thread_a).is_some() || sched::live_threads() != threads_before {
        return Err("cross-process thread records did not retire");
    }

    // A new process/thread must receive the canonical clean image after the
    // previous FP-using process was destroyed and its stable slot was reused.
    let fresh = prepare_process("m15Fresh", 1)?;
    let fresh_thread = spawn_fixture_thread(fresh, 0, 1)?;
    if sched::slot_of_thread(fresh_thread) != Some(reused_slot) {
        return Err("fresh process did not reuse the retired scheduler slot");
    }
    let fresh_switches = drive_tasks(&[(fresh_thread, EXIT_OK)])?;
    check_exit_statuses(&[(fresh_thread, EXIT_OK)])?;
    destroy_process(fresh)?;
    if frames::free_frames() != free_before || sched::live_threads() != threads_before {
        return Err("cross-process teardown or fresh-slot reuse leaked resources");
    }
    info!(
        "m15",
        "process isolation: distinct CR3 roots retained independent FP state ({switches} timer switches); fresh process entered cleanly in recycled slot {reused_slot} ({fresh_switches} switches), teardown exact"
    );
    Ok(())
}

fn test_fault_containment() -> Result<(), &'static str> {
    let mut all_faults_contained = true;
    for (kind, vector) in [(1u64, 19u64), (2, 16), (3, 6)] {
        let free_before = frames::free_frames();
        let fault_process = prepare_process("m15Fault", 2)?;
        let peer_process = prepare_process("m15FaultPeer", 1)?;
        let fault = spawn_fixture_thread(fault_process, 0, ARG_FAULT | kind)?;
        let sibling = spawn_fixture_thread(fault_process, 1, 1)?;
        let peer = spawn_fixture_thread(peer_process, 0, 2)?;
        let fault_status = 0x100 + vector;
        let switches = drive_tasks(&[(fault, fault_status), (sibling, EXIT_OK), (peer, EXIT_OK)])?;
        let contained = if let Err(reason) =
            check_exit_statuses(&[(fault, fault_status), (sibling, EXIT_OK), (peer, EXIT_OK)])
        {
            error!(
                "m15",
                "fault kind {kind} (vector {vector:#x}) was not contained with exact status: {reason}; observed {:?}",
                syscall::exit_status_of(fault)
            );
            all_faults_contained = false;
            false
        } else {
            true
        };
        destroy_process(fault_process)?;
        destroy_process(peer_process)?;
        if frames::free_frames() != free_before {
            error!("m15", "fault kind {kind} teardown leaked frames");
            all_faults_contained = false;
        }
        if contained {
            info!(
                "m15",
                "contained CPL3 vector {vector:#x}: fault status {fault_status:#x}; sibling and separate process survived ({switches} timer switches)"
            );
        } else {
            info!(
                "m15",
                "fault trial vector {vector:#x}: fault thread={:?}, sibling={:?}, peer={:?}; survivor/teardown checks completed ({switches} timer switches)",
                syscall::exit_status_of(fault),
                syscall::exit_status_of(sibling),
                syscall::exit_status_of(peer),
            );
        }
    }
    if all_faults_contained {
        Ok(())
    } else {
        Err("one or more CPL3 FP/SIMD or unsupported-instruction faults were not contained")
    }
}
