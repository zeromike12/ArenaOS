//! Phase 8.1 ring-3 configuration service: READ boundary only (ADR-0046).
//! Boot grants: 0=fsd Endpoint/WRITE, 1=config Endpoint/READ,
//! 2=inert diagnostic Notification/READ. Only a separate trusted actor
//! may eventually carry and transfer the matching update marker.
//! This checkpoint implements actual FS-backed, fail-closed READ; a
//! correctly marked SET is explicitly NOT_READY, not a fake commit.
#![no_std]
#![no_main]
use core::panic::PanicInfo;
#[path = "../../abi.rs"]
mod abi;
#[path = "../../config.rs"]
#[allow(dead_code)] // encode/next become live with the later authorized SET checkpoint
mod config;
use abi::*;

const FS: u64 = 0;
const SERVER: u64 = 1;
const MARKER: u64 = 2;
const BUFFER: u64 = 8;
const BUFFER_LENT: u64 = 9;

fn log(s: &str) {
    let _ = unsafe { syscall2(SYS_DEBUG_WRITE, s.as_ptr() as u64, s.len() as u64) };
}
fn fail(s: &str) -> ! {
    log(s);
    unsafe { syscall1(SYS_THREAD_EXIT, 86) };
    loop {
        core::hint::spin_loop()
    }
}
#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    fail("configd: PANIC\r\n")
}

fn call_fs(op: u64, w1: u64, cap: u64, msg: &mut [u8; MSG_BYTES]) -> Result<[u64; 3], u64> {
    let mut reply = [0u64; 3];
    let rc = unsafe {
        syscall6(
            SYS_IPC_CALL,
            FS,
            op,
            w1,
            cap,
            reply.as_mut_ptr() as u64,
            msg.as_mut_ptr() as u64,
        )
    };
    if rc < 0 || reply[2] != CAP_NONE {
        return Err(CFG_IO);
    }
    if reply[0] != FS_OK {
        return Err(if reply[0] == FS_ERR_CORRUPT {
            CFG_CORRUPT
        } else {
            CFG_IO
        });
    }
    Ok(reply)
}

/// One complete walk; never return a cached predecessor if ANY read
/// or validation fails. Each OPEN handle is closed even on short read.
fn scan_store(buffer_va: u64) -> Result<config::Recovery, u64> {
    let mut scan = config::Scan::new();
    let mut cursor = 0u32;
    for _ in 0..=32 {
        let mut dirent = [0u8; MSG_BYTES];
        let reply = call_fs(FS_OP_LS, u64::from(cursor), CAP_NONE, &mut dirent)?;
        let next = u32::from_le_bytes(dirent[..4].try_into().unwrap());
        if next == FS_CURSOR_END {
            return scan.finish().map_err(|_| CFG_CORRUPT);
        }
        let nlen = u32::from_le_bytes(dirent[12..16].try_into().unwrap()) as usize;
        let size = u64::from_le_bytes(dirent[4..12].try_into().unwrap());
        if next <= cursor
            || next > 32
            || nlen == 0
            || nlen >= FS_NAME_MAX
            || reply[1] >= 32
            || next as u64 != reply[1] + 1
        {
            return Err(CFG_CORRUPT);
        }
        let name = &dirent[16..16 + nlen];
        if name.starts_with(b"cfg8-") {
            if size == 0 {
                scan.ingest(name, size, None).map_err(|_| CFG_CORRUPT)?;
            } else {
                if size != 512 {
                    return Err(CFG_CORRUPT);
                }
                let mut arg = [0u8; MSG_BYTES];
                arg[..nlen].copy_from_slice(name);
                let handle = call_fs(FS_OP_OPEN, 0, CAP_NONE, &mut arg)?[1];
                arg = [0; MSG_BYTES];
                arg[..8].copy_from_slice(&512u64.to_le_bytes());
                let read = call_fs(FS_OP_READ, fs_rw_w1(handle, 0), BUFFER_LENT, &mut arg);
                let mut close_msg = [0u8; MSG_BYTES];
                let close = call_fs(FS_OP_CLOSE, handle, CAP_NONE, &mut close_msg);
                if read?.get(1) != Some(&512) || close.is_err() {
                    return Err(CFG_IO);
                }
                // SAFETY: mapped, owned reusable buffer; exactly 512
                // bytes were DMA'd into it before the FS call returned.
                let mut record = [0u8; 512];
                unsafe {
                    core::ptr::copy_nonoverlapping(buffer_va as *const u8, record.as_mut_ptr(), 512)
                };
                scan.ingest(name, size, Some(&record))
                    .map_err(|_| CFG_CORRUPT)?;
            }
        }
        cursor = next;
    }
    Err(CFG_CORRUPT) // no terminator after the maximum 32 objects
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    log("configd: READ boundary boot (no update implementation)\r\n");
    // One stable, self-mapped DMA frame. A LENT copy, not the mapped
    // owner, travels through fsd and storaged. No per-request leak.
    if unsafe { syscall1(SYS_ALLOC_FRAME, BUFFER) } <= 0
        || unsafe { syscall3(SYS_CAP_COPY, BUFFER, BUFFER_LENT, RIGHTS_ALL) } != 0
    {
        fail("configd: buffer frame refused\r\n");
    }
    let va = unsafe { syscall2(SYS_MAP_MEMORY, BUFFER, 1) };
    if va <= 0 {
        fail("configd: buffer map refused\r\n");
    }
    loop {
        let mut args = [0u64; 3];
        let mut request = [0u8; MSG_BYTES];
        if unsafe {
            syscall3(
                SYS_IPC_RECV,
                SERVER,
                args.as_mut_ptr() as u64,
                request.as_mut_ptr() as u64,
            )
        } < 0
        {
            fail("configd: recv refused\r\n");
        }
        let (op, landed) = (args[0], args[2]);
        let mut answer = [0u8; MSG_BYTES];
        let (status, seq) = match op {
            CFG_OP_READ if landed == CAP_NONE => match scan_store(va as u64) {
                Ok(state) => match state.current {
                    Some(record) => {
                        answer[..2].copy_from_slice(&(record.bytes().len() as u16).to_le_bytes());
                        answer[2..2 + record.bytes().len()].copy_from_slice(record.bytes());
                        log("configd: READ validated committed value\r\n");
                        (CFG_OK, u64::from(record.sequence()))
                    }
                    None => {
                        log("configd: READ UNSET\r\n");
                        (CFG_UNSET, 0)
                    }
                },
                Err(e) => {
                    log("configd: READ fail-closed (corrupt or unavailable)\r\n");
                    (e, 0)
                }
            },
            CFG_OP_SET => {
                if take_diagnostic(landed, MARKER) {
                    // No hidden write path in a read-only milestone.
                    (CFG_NOT_READY, 0)
                } else {
                    log("configd: SET refused without receiving-service authority\r\n");
                    (CFG_DENIED, 0)
                }
            }
            _ => {
                if landed != CAP_NONE {
                    let _ = unsafe { syscall1(SYS_CAP_DESTROY, landed) };
                }
                (CFG_BAD_OP, 0)
            }
        };
        if unsafe {
            syscall5(
                SYS_IPC_REPLY,
                SERVER,
                status,
                seq,
                CAP_NONE,
                answer.as_ptr() as u64,
            )
        } < 0
        {
            fail("configd: reply refused\r\n");
        }
    }
}
