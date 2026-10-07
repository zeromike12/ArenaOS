//! Ordinary application service adapter: named function grants, own I/O,
//! caller endpoint and private pacing notification. No raw fsd capability.
use crate::{abi::*, service_wire::Frame};
/// The checked service endpoint is a per-session BadgedEndpoint in slot 1.
pub const SERVICE_ENDPOINT: u64 = 1;
/// ABI-v2 requests carry service authority in the endpoint cap's kernel badge,
/// not in an attenuated SharedRegion sent as a payload cap.
pub const FUNCTION: u64 = CAP_NONE;
pub const SURFACE: u64 = 2;
pub const CLOCK: u64 = 3;
pub const DIAGNOSTICS: u64 = 4;
pub(crate) const IPC_BUSY_RETRIES: usize = 512;
pub(crate) fn retry_after_busy(attempt: usize) -> Result<(), i64> {
    let now = now();
    let delay = (1_000u64 << attempt.min(5)) + (now & 0x7ff);
    idle(Some(now.saturating_add(delay))).map(|_| ())
}
pub fn exchange(frame: Frame, cap: u64) -> Result<(u64, Frame), i64> {
    exchange_words(frame, cap, [0; 2])
}
/// Test and service adapter for the two descriptive IPC words. They never
/// select a session; that identity comes only from the held badged endpoint.
pub fn exchange_words(frame: Frame, cap: u64, words: [u64; 2]) -> Result<(u64, Frame), i64> {
    for attempt in 0..=IPC_BUSY_RETRIES {
        let mut bytes = frame.encode().map_err(|_| -2)?;
        let mut out = [0, 0, CAP_NONE];
        let rc = unsafe {
            syscall6(
                SYS_IPC_CALL,
                SERVICE_ENDPOINT,
                words[0],
                words[1],
                cap,
                out.as_mut_ptr() as u64,
                bytes.as_mut_ptr() as u64,
            )
        };
        if out[2] != CAP_NONE {
            unsafe {
                syscall1(SYS_CAP_DESTROY, out[2]);
            }
            return Err(-2);
        }
        // A kernel queue refusal means this call was never accepted, so
        // retrying cannot duplicate a service-side mutation. Never retry a
        // server's status in out[0]: that request was already processed.
        if rc == STATUS_BUSY && attempt < IPC_BUSY_RETRIES {
            retry_after_busy(attempt)?;
            continue;
        }
        if rc != 0 {
            return Err(rc);
        }
        if out[0] != 0 {
            return Err(out[0] as i64);
        }
        return Ok((out[1], Frame::decode(&bytes).map_err(|_| -2)?));
    }
    Err(STATUS_BUSY)
}
/// `SYS_CAP_DESCRIBE` of one of this process's slots.
///
/// # Safety
/// `out` is this process's own memory (always true for a reference).
pub unsafe fn describe_raw(slot: u64, out: &mut [u64; 3]) -> i64 {
    unsafe { syscall2(SYS_CAP_DESCRIBE, slot, out.as_mut_ptr() as u64) }
}
/// Offer filesd capability `cap` for the next application this session
/// launches (Files opening a document in the Editor). The broker receives
/// a copy; this process drops its own slot either way.
pub fn offer(cap: u64) -> Result<(), i64> {
    let r = exchange(Frame::Offer, cap).map(|_| ());
    unsafe {
        syscall1(SYS_CAP_DESTROY, cap);
    }
    r
}
/// Ask the broker to dispatch this exact File capability using the current
/// application registry. The app retains no copy after the request. Open
/// With makes the transferred cap read-only at the target boundary.
pub fn open_document(name: [u8; 32], cap: u64, open_with: bool) -> Result<(), i64> {
    let frame = if open_with {
        Frame::OpenWith { name }
    } else {
        Frame::OpenDocument { name }
    };
    let result = exchange(frame, cap).map(|_| ());
    unsafe {
        syscall1(SYS_CAP_DESTROY, cap);
    }
    result
}
/// What the trusted chooser granted.
pub struct Grant {
    /// The capability slot (this process's own copy).
    pub cap: u64,
    /// Display title only, never authority.
    pub title: [u8; 32],
    pub save: bool,
    pub read_only: bool,
}
/// The chooser's outcome, or None when the user cancelled.
pub fn take_grant() -> Result<Option<Grant>, i64> {
    let mut bytes = Frame::TakeGrant.encode().map_err(|_| -2)?;
    let mut out = [0, 0, CAP_NONE];
    let rc = unsafe {
        syscall6(
            SYS_IPC_CALL,
            SERVICE_ENDPOINT,
            0,
            0,
            FUNCTION,
            out.as_mut_ptr() as u64,
            bytes.as_mut_ptr() as u64,
        )
    };
    let reply = Frame::decode(&bytes);
    match (rc, out[0], reply) {
        (
            0,
            0,
            Ok(Frame::Granted {
                save,
                read_only,
                name,
            }),
        ) if out[2] != CAP_NONE => Ok(Some(Grant {
            cap: out[2],
            title: name,
            save,
            read_only,
        })),
        (0, 0, Ok(Frame::TakeGrant)) if out[2] == CAP_NONE => Ok(None),
        _ => {
            if out[2] != CAP_NONE {
                unsafe {
                    syscall1(SYS_CAP_DESTROY, out[2]);
                }
            }
            Err(if rc != 0 { rc } else { -2 })
        }
    }
}
pub fn startup() -> Result<(u8, bool, bool, [u8; 32]), i64> {
    // `exchange` retries only mutation-free kernel queue refusals with
    // bounded timer backoff; a server status is returned without retry.
    match exchange(Frame::Bootstrap, FUNCTION)? {
        (
            _,
            Frame::Started {
                kind,
                theme,
                motion,
                path,
            },
        ) => Ok((kind, theme == 1, motion, path)),
        _ => Err(-2),
    }
}

/// Request one helper declared by the installed application's signed AHL1
/// resource. The returned value is an opaque per-instance process-group
/// handle, never a PID or Process capability.
pub fn spawn_helper(helper_id: [u8; 32]) -> Result<u32, i64> {
    match exchange(Frame::SpawnHelper { helper_id }, CAP_NONE)? {
        (handle, Frame::HelperStarted { handle: returned })
            if handle == u64::from(returned) && returned != 0 =>
        {
            Ok(returned)
        }
        _ => Err(-2),
    }
}

/// Launch one signed helper with the AHL1 stream grant and receive its exact
/// parent-side stream region.
pub fn spawn_helper_streams(
    helper_id: [u8; 32],
) -> Result<(u32, arena_runtime::streams::HelperStreams), i64> {
    let (reply_word, reply, stream_cap) =
        exchange_with_reply_cap(Frame::SpawnHelperStreams { helper_id }, CAP_NONE)?;
    let returned = match reply {
        Frame::HelperStarted { handle } if u64::from(handle) == reply_word && handle != 0 => handle,
        _ => {
            unsafe { syscall1(SYS_CAP_DESTROY, stream_cap) };
            return Err(STATUS_BAD_ARG);
        }
    };
    let streams =
        arena_runtime::streams::HelperStreams::from_cap(stream_cap).map_err(
            |error| match error {
                arena_runtime::streams::Error::Kernel(status) => status,
                _ => STATUS_BAD_ARG,
            },
        )?;
    Ok((returned, streams))
}

/// Ask the authenticated Desktop app manager to wake one stream-enabled
/// helper owned by this AppInstance. The descriptive handle is checked only
/// after the caller's badged service capability authenticates the owner.
pub fn wake_helper(handle: u32) -> Result<(), i64> {
    if handle == 0 {
        return Err(STATUS_BAD_ARG);
    }
    match exchange(Frame::WakeHelper { handle }, CAP_NONE)? {
        (0, Frame::HelperWoken { handle: returned }) if returned == handle => Ok(()),
        _ => Err(STATUS_BAD_ARG),
    }
}

/// Checked IPC call that accepts exactly one explicitly returned capability.
/// The ordinary `exchange` API continues to reject unexpected cap transfer.
fn exchange_with_reply_cap(frame: Frame, cap: u64) -> Result<(u64, Frame, u64), i64> {
    for attempt in 0..=IPC_BUSY_RETRIES {
        let mut bytes = frame.encode().map_err(|_| STATUS_BAD_ARG)?;
        let mut out = [0, 0, CAP_NONE];
        let rc = unsafe {
            syscall6(
                SYS_IPC_CALL,
                SERVICE_ENDPOINT,
                0,
                0,
                cap,
                out.as_mut_ptr() as u64,
                bytes.as_mut_ptr() as u64,
            )
        };
        if rc == STATUS_BUSY && attempt < IPC_BUSY_RETRIES {
            if out[2] != CAP_NONE {
                unsafe { syscall1(SYS_CAP_DESTROY, out[2]) };
                return Err(STATUS_BAD_ARG);
            }
            retry_after_busy(attempt)?;
            continue;
        }
        if rc != 0 {
            if out[2] != CAP_NONE {
                unsafe { syscall1(SYS_CAP_DESTROY, out[2]) };
            }
            return Err(rc);
        }
        if out[0] != 0 {
            if out[2] != CAP_NONE {
                unsafe { syscall1(SYS_CAP_DESTROY, out[2]) };
            }
            return Err(out[0] as i64);
        }
        let decoded = match Frame::decode(&bytes) {
            Ok(frame) if out[2] != CAP_NONE => frame,
            _ => {
                if out[2] != CAP_NONE {
                    unsafe { syscall1(SYS_CAP_DESTROY, out[2]) };
                }
                return Err(STATUS_BAD_ARG);
            }
        };
        return Ok((out[1], decoded, out[2]));
    }
    Err(STATUS_BUSY)
}

/// Wait for a helper's exact Process-cap exit status. The Desktop polls
/// without blocking its global event loop; this client waits between polls
/// using its own explicitly held notification/timer authority.
pub fn wait_helper(handle: u32) -> Result<u64, i64> {
    if handle == 0 {
        return Err(-2);
    }
    loop {
        match exchange(Frame::WaitHelper { handle }, CAP_NONE) {
            Ok((
                status,
                Frame::HelperExited {
                    handle: returned,
                    status: returned_status,
                },
            )) if returned == handle && status == returned_status => return Ok(status),
            Err(error) if error == STATUS_BUSY => {
                idle(Some(now().saturating_add(10_000)))?;
            }
            Ok(_) => return Err(-2),
            Err(error) => return Err(error),
        }
    }
}

/// Stop and reap a helper owned by this AppInstance. The handle is checked
/// against the caller's badge-bound group by Desktop before any Process
/// operation occurs.
pub fn terminate_helper(handle: u32) -> Result<(), i64> {
    if handle == 0 {
        return Err(-2);
    }
    match exchange(Frame::TerminateHelper { handle }, CAP_NONE)? {
        (0, Frame::HelperTerminated { handle: returned }) if returned == handle => Ok(()),
        _ => Err(-2),
    }
}
/// Sleep until the broker signals queued events on this client's private
/// clock, or until `deadline` (monotonic microseconds) when the client has
/// time-driven work (Monitor sampling). A client with nothing scheduled
/// arms no timer at all and costs zero wakes while idle (ADR-0071). An
/// early wake cancels the still-armed timer so armed timers never
/// accumulate against the per-process quota; cancelling an already-fired
/// timer is a harmless refusal.
/// A directory watch fired (ADR-0079): a hint on the clock, confirmed
/// through the watched record.
pub const WATCH_BADGE: u64 = 2;
/// Native stream state changed (input arrived or output space was reclaimed).
pub const STREAM_BADGE: u64 = 4;
/// Sleep until the broker signals (badge 1), a directory watch fires
/// (badge 2), standard stream state changes (badge 4), or the deadline
/// passes. Returns the merged badge.
pub fn idle(deadline: Option<u64>) -> Result<u64, i64> {
    let id = match deadline {
        Some(at) => {
            let delay = at.saturating_sub(now()).max(1);
            let id = unsafe { syscall3(SYS_TIMER_ARM, CLOCK, 1, delay) };
            if id < 0 {
                return Err(id);
            }
            Some(id)
        }
        None => None,
    };
    let badge = unsafe { syscall1(SYS_WAIT, CLOCK) };
    if let Some(id) = id {
        let _ = unsafe { syscall1(SYS_TIMER_CANCEL, id as u64) };
    }
    if badge < 0 {
        return Err(badge);
    }
    let badge = badge as u64;
    if badge == 0 || badge & !(1 | WATCH_BADGE | STREAM_BADGE) != 0 {
        return Err(-2);
    }
    Ok(badge)
}
pub fn observe() -> Result<[u64; 9], i64> {
    let mut counts = [0u64; 9];
    let rc = unsafe {
        syscall6(
            SYS_OBSERVE,
            DIAGNOSTICS,
            counts.as_mut_ptr() as u64,
            0,
            0,
            0,
            0,
        )
    };
    if rc != 0 { Err(rc) } else { Ok(counts) }
}
pub fn processes(out: &mut [(u64, u64); 32]) -> Result<usize, i64> {
    let n = unsafe { syscall2(SYS_PROC_LIST, out.as_mut_ptr() as u64, 32) };
    if n < 0 { Err(n) } else { Ok(n as usize) }
}
pub fn now() -> u64 {
    unsafe { syscall0(SYS_CLOCK_NOW) }.max(0) as u64
}
/// Exercise the actual delegated boundary before showing an application.
/// These checks use native calls/IPC, not names or assertions about source.
pub fn audit(kind: u8, instance_generation: u64) -> Result<(), i64> {
    let mut endpoint = [0u64; 3];
    let mut surface = [0u64; 3];
    let mut clock = [0u64; 3];
    if kind > 6
        || unsafe { syscall6(SYS_CAP_OCCUPIED, 0, 0, 0, 0, 0, 0) } != 0
        || unsafe { syscall2(SYS_CAP_DESCRIBE, SERVICE_ENDPOINT, endpoint.as_mut_ptr() as u64) }
            != 0
        || endpoint[0] != 12
        || endpoint[2] != (RIGHTS_WRITE | RIGHTS_COPY | RIGHTS_DESTROY)
        || unsafe { syscall2(SYS_CAP_DESCRIBE, SURFACE, surface.as_mut_ptr() as u64) } != 0
        || surface[0] != 7
        || surface[2] != (RIGHTS_READ | RIGHTS_WRITE | RIGHTS_COPY)
        || unsafe { syscall2(SYS_CAP_DESCRIBE, CLOCK, clock.as_mut_ptr() as u64) } != 0
        || clock[0] != 3
        // Files may lend its clock for its folder watch (COPY).
        || clock[2] != if kind == 1 { 7 } else { 3 }
    {
        return Err(-2);
    }
    // A client-side BadgedEndpoint is not a server-side plain endpoint and
    // cannot mint a sibling badge. The kernel rejects this before any cap is
    // installed; numeric words below are not a substitute.
    let forged = unsafe {
        syscall6(
            SYS_ENDPOINT_MINT,
            SERVICE_ENDPOINT,
            0xA12C_0042,
            RIGHTS_WRITE | RIGHTS_COPY | RIGHTS_DESTROY,
            0,
            0,
            0,
        )
    };
    if forged >= 0 {
        if forged as u64 != CAP_NONE {
            unsafe { syscall1(SYS_CAP_DESTROY, forged as u64) };
        }
        return Err(-2);
    }
    if instance_generation == 0 {
        return Err(-2);
    }
    // For every launch after the first, the previous monotonic generation is
    // another live session's badge value in the scale test. Placing it in
    // caller-controlled words must still bootstrap this endpoint's own kind.
    let previous_badge_word = instance_generation.saturating_sub(1);
    match exchange_words(Frame::Bootstrap, FUNCTION, [previous_badge_word, u64::MAX]) {
        Ok((_, Frame::Started { kind: observed, .. })) if observed == kind => {}
        _ => return Err(-2),
    }
    // The endpoint itself is not a surface or function grant. A valid badge
    // with the wrong attached object must be refused without a service effect.
    if exchange_words(Frame::Display, SERVICE_ENDPOINT, [0; 2]).is_ok() {
        return Err(-2);
    }
    if kind != 3
        && exchange_words(
            Frame::Configure {
                theme: 1,
                motion: false,
            },
            FUNCTION,
            [u64::MAX, u64::MAX],
        )
        .is_ok()
    {
        return Err(-2);
    }
    let mut private = [0u8; 32];
    private[..10].copy_from_slice(b"ui10-prefs");
    if exchange_words(
        Frame::Read { name: private },
        FUNCTION,
        [u64::MAX, u64::MAX],
    )
    .is_ok()
    {
        return Err(-2);
    }
    if kind != 0
        && exchange_words(
            Frame::Launch {
                kind: 3,
                path: [0; 32],
            },
            FUNCTION,
            [u64::MAX, u64::MAX],
        )
        .is_ok()
    {
        return Err(-2);
    }
    let mut out = [0u64; 9];
    if unsafe { syscall6(SYS_OBSERVE, 1, out.as_mut_ptr() as u64, 0, 0, 0, 0) } >= 0 {
        return Err(-2);
    }
    if kind == 4 {
        let counts = observe()?;
        if counts[0] == 0
            || counts[0] >= counts[1]
            || counts[3] == 0
            || counts[4] < 2
            || counts[5] < 127
            || counts[7] != 4
        {
            return Err(-2);
        }
        if unsafe {
            syscall6(
                SYS_OBSERVE,
                DIAGNOSTICS,
                out.as_mut_ptr() as u64,
                1,
                0,
                0,
                0,
            )
        } >= 0
            || unsafe { syscall6(SYS_OBSERVE, DIAGNOSTICS, 0, 0, 0, 0, 0) } >= 0
            || unsafe {
                syscall6(
                    SYS_SHARED_CREATE,
                    DIAGNOSTICS,
                    1,
                    out.as_mut_ptr() as u64,
                    0,
                    0,
                    0,
                )
            } >= 0
        {
            return Err(-2);
        }
    }
    let marker = b"[application] ABI-v2 badge dispatch audit PASS\n";
    unsafe {
        syscall2(SYS_DEBUG_WRITE, marker.as_ptr() as u64, marker.len() as u64);
    }
    Ok(())
}
