//! Thin, raw syscall boundary. IPC's wire checks live in `ipc`, not here.
use crate::abi::{syscall6, SYS_IPC_CALL, MSG_BYTES};

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
