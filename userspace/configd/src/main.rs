//! Phase 8.1 ring-3 bounded transactional configuration service (ADR-0046).
//! Boot grants: 0=fsd Endpoint/WRITE, 1=config Endpoint/READ,
//! 2=update marker Notification/READ. The separate trusted updater
//! transfers the exact service-issued marker on every destructive call.
//! Writes are immutable generations under AFS1's atomic-sector model.
#![no_std]
#![no_main]
use core::panic::PanicInfo;
#[path = "../../abi.rs"]
mod abi;
#[path = "../../config.rs"]
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
        return Err(match reply[0] {
            FS_ERR_CORRUPT => CFG_CORRUPT,
            FS_ERR_NO_SPACE => CFG_NO_SPACE,
            _ => CFG_IO,
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

/// Bounded opt-in plan, not authority: a raw-FS trusted shell writes
/// exactly one intent file on the *previous* boot. The updater still
/// needs to transfer its service-issued marker on every subsequent SET.
fn scan_plan() -> Result<u64, u64> {
    let mut cursor = 0u32;
    let mut plan = 0u64;
    for _ in 0..=32 {
        let mut entry = [0u8; MSG_BYTES];
        let reply = call_fs(FS_OP_LS, u64::from(cursor), CAP_NONE, &mut entry)?;
        let next = u32::from_le_bytes(entry[..4].try_into().unwrap());
        if next == FS_CURSOR_END {
            return Ok(plan);
        }
        let n = u32::from_le_bytes(entry[12..16].try_into().unwrap()) as usize;
        let size = u64::from_le_bytes(entry[4..12].try_into().unwrap());
        if next <= cursor
            || next > 32
            || n == 0
            || n >= FS_NAME_MAX
            || reply[1] >= 32
            || next as u64 != reply[1] + 1
        {
            return Err(CFG_CORRUPT);
        }
        let name = &entry[16..16 + n];
        if name.starts_with(b"cfg-intent-") {
            let value = match name {
                b"cfg-intent-one" => 1,
                b"cfg-intent-two" => 2,
                _ => return Err(CFG_BAD_INPUT),
            };
            if plan != 0 || size == 0 || size > 32 {
                return Err(CFG_BAD_INPUT);
            }
            plan = value;
        }
        cursor = next;
    }
    Err(CFG_CORRUPT)
}

/// One atomic version transition; callers MUST have already delivered
/// and consumed the exact update marker at the receiving boundary.
fn update(
    request: &[u8; MSG_BYTES],
    buffer_va: u64,
    started: &mut bool,
) -> Result<(u64, u64), u64> {
    let len = u16::from_le_bytes([request[0], request[1]]) as usize;
    if len > config::PAYLOAD_MAX || request[2 + len..].iter().any(|&b| b != 0) {
        return Err(CFG_BAD_INPUT);
    }
    let payload = &request[2..2 + len];
    let state = scan_store(buffer_va)?;
    if state.pending.is_none() {
        if let Some(current) = state.current {
            if current.bytes() == payload {
                return Ok((CFG_UNCHANGED, u64::from(current.sequence())));
            }
        }
    }
    let Some(seq) = state.next() else {
        // Distinguish bounded generation exhaustion (w1=8) from a
        // physical FS allocator refusal (w1=0) without inventing a
        // ninth name or treating disk-full as a committed update.
        return Ok((CFG_NO_SPACE, u64::from(config::MAX_GENERATIONS)));
    };
    let mut record = [0u8; config::SECTOR_BYTES];
    config::encode(seq, payload, &mut record).map_err(|_| CFG_BAD_INPUT)?;
    let name = config::name(seq).map_err(|_| CFG_BAD_INPUT)?;
    let mut handle = None;
    // All failures after this point may have altered fsd's disk state,
    // including a CREATE that committed but returned a handle error.
    *started = true;
    if state.pending.is_none() {
        let mut msg = [0u8; MSG_BYTES];
        msg[..name.len()].copy_from_slice(&name);
        log("configd: SET CREATE submitted\r\n");
        handle = Some(call_fs(FS_OP_CREATE, 0, CAP_NONE, &mut msg)?[1]);
        log("configd: SET CREATE committed empty generation\r\n");
    }
    if handle.is_none() {
        let mut msg = [0u8; MSG_BYTES];
        msg[..name.len()].copy_from_slice(&name);
        handle = Some(call_fs(FS_OP_OPEN, 0, CAP_NONE, &mut msg)?[1]);
        log("configd: SET resumed committed empty generation\r\n");
    }
    let fh = handle.unwrap();
    // SAFETY: owned mapped 4 KiB frame and a validated 512-byte record;
    // only the LENT copy is passed to fsd; old generations are untouched.
    unsafe { core::ptr::copy_nonoverlapping(record.as_ptr(), buffer_va as *mut u8, 512) };
    let mut msg = [0u8; MSG_BYTES];
    msg[..8].copy_from_slice(&512u64.to_le_bytes());
    log("configd: SET WRITE submitted\r\n");
    let write = call_fs(FS_OP_WRITE, fs_rw_w1(fh, 0), BUFFER_LENT, &mut msg);
    if write.as_ref().is_ok_and(|r| r[1] == 512) {
        log("configd: SET WRITE committed (fsd replied)\r\n");
    }
    let mut close_msg = [0u8; MSG_BYTES];
    let close = call_fs(FS_OP_CLOSE, fh, CAP_NONE, &mut close_msg);
    if write?.get(1) != Some(&512) || close.is_err() {
        return Err(CFG_IO);
    }
    log("configd: SET CLOSE completed\r\n");
    let post = scan_store(buffer_va)?;
    if post.pending.is_some()
        || post
            .current
            .is_none_or(|r| r.sequence() != seq || r.bytes() != payload)
    {
        return Err(CFG_CORRUPT);
    }
    log("configd: SET exact new value verified before reply\r\n");
    Ok((CFG_COMMITTED, u64::from(seq)))
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    log("configd: transactional store boot; marker-gated updates\r\n");
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
    let mut degraded = false;
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
            CFG_OP_TEST_PLAN => {
                if !take_diagnostic(landed, MARKER) {
                    (CFG_DENIED, 0)
                } else {
                    match scan_plan() {
                        Ok(0) => (CFG_UNSET, 0),
                        Ok(n) => (CFG_OK, n),
                        Err(e) => (e, 0),
                    }
                }
            }
            CFG_OP_SET => {
                if !take_diagnostic(landed, MARKER) {
                    log("configd: SET refused without receiving-service authority\r\n");
                    (CFG_DENIED, 0)
                } else if degraded {
                    (CFG_DEGRADED, 0)
                } else {
                    let mut started = false;
                    match update(&request, va as u64, &mut started) {
                        Ok(done) => done,
                        Err(e) => {
                            // Exhausted immutable slots and invalid caller
                            // bytes require no FS mutation. An FS_NO_SPACE
                            // after CREATE/OPEN *is* ambiguous; latch the
                            // write barrier even though its reply is typed.
                            if started || (e != CFG_BAD_INPUT && e != CFG_NO_SPACE) {
                                degraded = true;
                            }
                            (e, 0)
                        }
                    }
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
