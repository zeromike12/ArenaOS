//! Trusted Desktop broker client for the narrow packaged APB1 operation.
//!
//! The broker supplies one exact filesd capability for the selected source
//! file. The file capability is input data, never install approval; packaged
//! resolves receiver-owned APKG policy and filesd owns AFS2 mutation.

use crate::abi::*;

/// Kernel-issued, write-only package receiver endpoint (ADR-0091).
pub const ENDPOINT_SLOT: u64 = 43;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstallReceipt {
    /// Descriptive signed APB1 version; not launch authority.
    pub version: u64,
    /// Whole-bundle digest returned after durable filesd readback.
    pub bundle_digest: [u8; 32],
}

/// Submit an exact filesd File capability to packaged. The broker retains its
/// original cap; IPC transfers a copy and the receiving services re-check the
/// endpoint badge, File record, signer policy, digest and AFS2 transaction.
pub fn install_apb1(source_file: u64) -> Result<InstallReceipt, u64> {
    let mut package = [0u64; 3];
    let mut source = [0u64; 3];
    if source_file == CAP_NONE
        || unsafe { syscall2(SYS_CAP_DESCRIBE, ENDPOINT_SLOT, package.as_mut_ptr() as u64) } != 0
        || package[0] != 2
        || package[2] != RIGHTS_WRITE
        || unsafe { syscall2(SYS_CAP_DESCRIBE, source_file, source.as_mut_ptr() as u64) } != 0
        || source[0] != 12
        || source[2] & (RIGHTS_WRITE | RIGHTS_COPY) != RIGHTS_WRITE | RIGHTS_COPY
    {
        return Err(PKG_DENY);
    }

    let mut request = [0u8; 64];
    let mut reply = [0u64, 0, CAP_NONE];
    let transport = unsafe {
        syscall6(
            SYS_IPC_CALL,
            ENDPOINT_SLOT,
            PKG_OP_APB1_INSTALL,
            0,
            source_file,
            reply.as_mut_ptr() as u64,
            request.as_mut_ptr() as u64,
        )
    };
    if transport != 0 {
        if reply[2] != CAP_NONE {
            let _ = unsafe { syscall1(SYS_CAP_DESTROY, reply[2]) };
        }
        return Err(PKG_OFFLINE);
    }
    if reply[2] != CAP_NONE {
        let _ = unsafe { syscall1(SYS_CAP_DESTROY, reply[2]) };
        return Err(PKG_CORRUPT);
    }
    if reply[0] != PKG_INSTALLED {
        return Err(reply[0]);
    }
    if reply[1] == 0 {
        return Err(PKG_CORRUPT);
    }
    let mut bundle_digest = [0u8; 32];
    bundle_digest.copy_from_slice(&request[..32]);
    Ok(InstallReceipt {
        version: reply[1],
        bundle_digest,
    })
}
