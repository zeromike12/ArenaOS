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
pub fn idle() -> Result<(), i64> {
    let id = unsafe { syscall3(SYS_TIMER_ARM, CLOCK, 1, arena_ui::motion::FRAME_US) };
    if id < 0 {
        return Err(id);
    }
    let badge = unsafe { syscall1(SYS_WAIT, CLOCK) };
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
