//! Thin, raw syscall boundary. IPC's wire checks live in `ipc`, not here.
use crate::abi::{syscall1, syscall6, SYS_CAP_DESTROY, SYS_IPC_CALL, MSG_BYTES};

/// Exactly the existing six-argument ABI. Slot possession, not this
/// wrapper, authorizes the kernel call. No retry (non-idempotent ops).
pub fn ipc_call(
    endpoint: u64, w0: u64, w1: u64, attached: u64,
    msg: &mut [u8; MSG_BYTES],
) -> Result<[u64; 3], i64> {
    let mut reply = [0u64; 3];
    let rc = unsafe {
        syscall6(SYS_IPC_CALL, endpoint, w0, w1, attached,
                 reply.as_mut_ptr() as u64, msg.as_mut_ptr() as u64)
    };
    if rc < 0 { Err(rc) } else { Ok(reply) }
}

/// Discard a reply cap that IPC *already landed* in this process. The
/// kernel's IPC provenance permits discarding even an attenuated cap
/// without DESTROY rights; do not turn a failed discard into a claim
/// that the unexpected authority was contained.
pub fn discard_landed_cap(slot: u64) -> Result<(), i64> {
    let rc = unsafe { syscall1(SYS_CAP_DESTROY, slot) };
    if rc != 0 { Err(rc) } else { Ok(()) }
}
