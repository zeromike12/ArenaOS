//! Ordinary application service adapter: named function grants, own I/O,
//! caller endpoint and private pacing notification. No raw fsd capability.
use crate::{abi::*, service_wire::Frame};
pub const FUNCTION: u64 = 2;
pub const CLOCK: u64 = 3;
pub const DIAGNOSTICS: u64 = 4;
pub fn exchange(frame: Frame, cap: u64) -> Result<(u64, Frame), i64> {
    let mut bytes = frame.encode().map_err(|_| -2)?;
    let mut out = [0, 0, CAP_NONE];
    let rc = unsafe {
        syscall6(
            SYS_IPC_CALL,
            0,
            0,
            0,
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
    if rc != 0 {
        return Err(rc);
    }
    if out[0] != 0 {
        return Err(out[0] as i64);
    }
    Ok((out[1], Frame::decode(&bytes).map_err(|_| -2)?))
}
pub fn startup() -> Result<(u8, bool, bool, [u8; 32]), i64> {
    match exchange(Frame::Bootstrap, 1)?.1 {
        Frame::Started {
            kind,
            theme,
            motion,
            path,
        } => Ok((kind, theme == 1, motion, path)),
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
pub fn idle(deadline: Option<u64>) -> Result<(), i64> {
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
    if badge != 1 {
        return Err(-2);
    }
    Ok(())
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
pub fn audit(kind: u8) -> Result<(), i64> {
    let mut own = [0u64; 3];
    let mut function = [0u64; 3];
    let mut clock = [0u64; 3];
    if unsafe { syscall2(SYS_CAP_DESCRIBE, 1, own.as_mut_ptr() as u64) } != 0
        || own[0] != 7
        || own[2] != 7
        || unsafe { syscall2(SYS_CAP_DESCRIBE, FUNCTION, function.as_mut_ptr() as u64) } != 0
        || function[0] != 7
        || function[1] != own[1]
        || function[2]
            != match kind {
                0..=2 => 15,
                3 => 14,
                _ => 5,
            }
        || unsafe { syscall2(SYS_CAP_DESCRIBE, CLOCK, clock.as_mut_ptr() as u64) } != 0
        || clock[0] != 3
        || clock[2] != 3
    {
        return Err(-2);
    }
    if kind != 3
        && exchange(
            Frame::Configure {
                theme: 1,
                motion: false,
            },
            FUNCTION,
        )
        .is_ok()
    {
        return Err(-2);
    }
    let mut private = [0u8; 32];
    private[..10].copy_from_slice(b"ui10-prefs");
    if exchange(Frame::Read { name: private }, FUNCTION).is_ok() {
        return Err(-2);
    }
    if kind != 0
        && exchange(
            Frame::Launch {
                kind: 3,
                path: [0; 32],
            },
            FUNCTION,
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
            || counts[7] != 5
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
    Ok(())
}
