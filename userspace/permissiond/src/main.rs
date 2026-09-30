#![no_std]
#![no_main]
// ADR-0048 first integrated (volatile-only) path. No decision is persisted;
// every restart starts DENY. Never describe PERM_VOLATILE as durable approval.
#[path = "../../abi.rs"]
mod abi;
use abi::*;
use core::panic::PanicInfo;

// Exactly four spawn grants: 0 fsd WRITE, 1 rngd WRITE,
// 2 receiver READ, 3 admin marker READ. A separate
// bounded PING worker checks readiness; the broker gets no fifth grant.
// Slot 4 receives IPC transfers; 7/8 are owned+LENT scratch.
const FS: u64 = 0;
const RNG: u64 = 1;
const SERVER: u64 = 2;
const MARKER: u64 = 3;
const BUFFER: u64 = 7;
const BUFFER_LENT: u64 = 8;

fn fail(what: &str) -> ! {
    log_line(|o| { o.str("permissiond: "); o.str(what); o.crlf(); });
    unsafe { syscall1(SYS_THREAD_EXIT, 89) };
    loop { core::hint::spin_loop() }
}
#[panic_handler]
fn panic(_: &PanicInfo) -> ! { fail("PANIC") }

fn call(cap: u64, op: u64, w1: u64, lent: u64, msg: &mut [u8; MSG_BYTES]) -> Result<[u64; 3], ()> {
    let mut answer = [0u64; 3];
    let rc = unsafe { syscall6(SYS_IPC_CALL, cap, op, w1, lent,
        answer.as_mut_ptr() as u64, msg.as_mut_ptr() as u64) };
    if rc < 0 || answer[2] != CAP_NONE { return Err(()); }
    Ok(answer)
}

const MAX_TOKENS: usize = 4;
fn token_eq(a: &[u8], b: &[u8]) -> bool {
    let mut diff = 0u8;
    for i in 0..16 { diff |= a[i] ^ b[i]; }
    diff == 0
}
fn draw_token(va: u64) -> Result<[u8; 16], ()> {
    let mut msg = [0u8; MSG_BYTES];
    // rngd's wire order differs from fsd: byte count is w0, opcode w1.
    let r = call(RNG, 16, RNG_OP_GET, BUFFER_LENT, &mut msg)?;
    if r[0] != RNG_S_OK || r[1] != 16 { return Err(()); }
    let mut token = [0u8; 16];
    unsafe { core::ptr::copy_nonoverlapping(va as *const u8, token.as_mut_ptr(), 16) };
    Ok(token)
}

// The file is selected by the receiver, never by a caller-supplied path.
// A failure at ANY step cannot return bytes. Close even after a bad read.
fn read_file(va: u64, offset: u64, answer: &mut [u8; MSG_BYTES]) -> Result<u64, ()> {
    if offset >= 512 || offset % 32 != 0 { return Err(()); }
    let mut msg = [0u8; MSG_BYTES];
    msg[..9].copy_from_slice(b"arena.txt");
    let opened = call(FS, FS_OP_OPEN, 0, CAP_NONE, &mut msg)?;
    if opened[0] != FS_OK { return Err(()); }
    let fh = opened[1];
    msg = [0; MSG_BYTES];
    // AFS1's data-plane issues full-sector DMA. Read the entire 512-B
    // object at sector-aligned offset zero, then select the requested
    // 32-B reply window inside our mapped frame. Nonaligned FS offsets
    // currently fail in fsd's block-buffer offset arithmetic.
    msg[..8].copy_from_slice(&512u64.to_le_bytes());
    let result = call(FS, FS_OP_READ, fs_rw_w1(fh, 0), BUFFER_LENT, &mut msg);
    let mut close = [0; MSG_BYTES];
    let closed = call(FS, FS_OP_CLOSE, fh, CAP_NONE, &mut close);
    let got = result?;
    if got[0] != FS_OK || got[1] != 512 || !matches!(closed, Ok(c) if c[0] == FS_OK) {
        return Err(());
    }
    unsafe { core::ptr::copy_nonoverlapping((va + offset) as *const u8, answer.as_mut_ptr(), 32) };
    Ok(32)
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    if unsafe { syscall1(SYS_ALLOC_FRAME, BUFFER) } <= 0
        || unsafe { syscall3(SYS_CAP_COPY, BUFFER, BUFFER_LENT, RIGHTS_ALL) } != 0 {
        fail("buffer allocation refused");
    }
    let va = unsafe { syscall2(SYS_MAP_MEMORY, BUFFER, 1) };
    if va <= 0 { fail("buffer map refused"); }
    log_line(|o| o.str("permissiond: serving (VOLATILE, default DENY)"));
    let mut allowed = false;
    let mut tokens = [[0u8; 16]; MAX_TOKENS];
    let mut count = 0usize;
    let mut retired = [0u8; 16];
    let mut bad_ping = false;
    let mut stall_ping = false;
    loop {
        let mut req = [0u8; MSG_BYTES];
        let mut words = [0u64; 3];
        if unsafe { syscall3(SYS_IPC_RECV, SERVER, words.as_mut_ptr() as u64,
            req.as_mut_ptr() as u64) } < 0 { fail("recv refused"); }
        let (op, offset, landed) = (words[0], words[1], words[2]);
        let mut reply = [0u8; MSG_BYTES];
        let (status, value) = match op {
            PERM_OP_PING if landed == CAP_NONE && offset == 0 && req == [0; MSG_BYTES] => {
                if stall_ping {
                    log_line(|o| o.str("permissiond: diagnostic PING stalled after dispatch"));
                    // The manager's timer/Process cap must tear down
                    // this live receiver and its blocked caller.
                    let _ = unsafe { syscall1(SYS_WAIT, MARKER) };
                    fail("diagnostic PING wait unexpectedly woke");
                }
                if bad_ping {
                    bad_ping = false;
                    log_line(|o| o.str("permissiond: diagnostic PING sent wrong typed reply"));
                    (PERM_BAD_INPUT, 0)
                } else {
                    reply[..4].copy_from_slice(&PERM_PING_MAGIC);
                    (PERM_OK, PERM_PING_VERSION)
                }
            },
            PERM_OP_TEST_BAD_PING | PERM_OP_TEST_STALL_PING => {
                if !take_diagnostic(landed, MARKER) { (PERM_DENIED, 0) }
                else {
                    bad_ping = op == PERM_OP_TEST_BAD_PING;
                    stall_ping = op == PERM_OP_TEST_STALL_PING;
                    (PERM_OK, 0)
                }
            },
            PERM_OP_ALLOW | PERM_OP_DENY | PERM_OP_REVOKE => {
                if !take_diagnostic(landed, MARKER) {
                    (PERM_DENIED, 0)
                } else {
                    // Invalidation happens before an admin reply. Every copy
                    // of old token bytes fails the subsequent READ check.
                    if count != 0 { retired = tokens[count - 1]; }
                    tokens = [[0; 16]; MAX_TOKENS];
                    count = 0;
                    allowed = op == PERM_OP_ALLOW;
                    log_line(|o| { o.str("permissiond: authorized VOLATILE "); o.u64(op); });
                    (PERM_VOLATILE, 0)
                }
            }
            PERM_OP_ACQUIRE if landed == CAP_NONE => {
                if !allowed { (PERM_DENIED, 0) }
                else if count == MAX_TOKENS { (PERM_NO_SPACE, 0) }
                else {
                    // No entropy fallback and no eviction of an
                    // independently issued bearer to make room.
                    let fresh = draw_token(va as u64).and_then(|candidate| {
                        if candidate == [0; 16] || token_eq(&candidate, &retired)
                            || tokens[..count].iter().any(|t| token_eq(t, &candidate)) {
                            Err(())
                        } else { Ok(candidate) }
                    });
                    match fresh {
                        Ok(new) => {
                            tokens[count] = new;
                            count += 1;
                            reply[..16].copy_from_slice(&new);
                            (PERM_OK, 16)
                        }
                        Err(()) => (PERM_ENTROPY, 0),
                    }
                }
            }
            PERM_OP_READ if landed == CAP_NONE => {
                if !allowed || !tokens[..count].iter().any(|t| token_eq(t, &req[..16])) {
                    (PERM_BAD_TOKEN, 0)
                } else if offset >= 512 || offset % 32 != 0 {
                    (PERM_BAD_INPUT, 0)
                } else {
                    match read_file(va as u64, offset, &mut reply) {
                        Ok(len) => (PERM_OK, len),
                        Err(()) => (PERM_IO, 0),
                    }
                }
            }
            _ => {
                if landed != CAP_NONE { let _ = unsafe { syscall1(SYS_CAP_DESTROY, landed) }; }
                (PERM_BAD_INPUT, 0)
            }
        };
        if unsafe { syscall5(SYS_IPC_REPLY, SERVER, status, value, CAP_NONE,
            reply.as_ptr() as u64) } < 0 { fail("reply refused"); }
    }
}
