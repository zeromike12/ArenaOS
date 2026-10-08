//! ADR-0048: only broker writes the separate, immutable perm8-* namespace.
//! A positive admin acknowledgement is AFTER exact committed rescan.
//! No claim about malicious FS writers, commit-sector corruption or rollback.
use super::*;
use arena_lib::fs as client_fs;
#[path = "../../permission.rs"]
mod record;

pub(super) fn fs(op: u64, w1: u64, cap: u64, msg: &mut [u8; MSG_BYTES]) -> Result<[u64; 3], u64> {
    // The persisted-policy path uses the shared TYPED client: exactly
    // the same endpoint cap and LENT frame as before, with validation
    // before the syscall and no extra transaction or hidden retry.
    let client = client_fs::Client::new(FS);
    let result = match op {
        FS_OP_LS => client.list(w1, msg),
        FS_OP_OPEN | FS_OP_CREATE => {
            let len = msg.iter().position(|&b| b == 0).unwrap_or(MSG_BYTES);
            if op == FS_OP_OPEN {
                client.open(&msg[..len])
            } else {
                client.create(&msg[..len])
            }
        }
        FS_OP_READ | FS_OP_WRITE => {
            let len = u64::from_le_bytes(msg[..8].try_into().unwrap());
            let (fh, offset) = (w1 & 0xff, w1 >> 8);
            if op == FS_OP_READ {
                client.read(fh, offset, cap, len)
            } else {
                client.write(fh, offset, cap, len)
            }
        }
        FS_OP_CLOSE => client.close(w1),
        _ => return Err(PERM_IO),
    }
    .map_err(|_| PERM_IO)?;
    if result.status != FS_OK {
        return Err(match result.status {
            FS_ERR_CORRUPT => PERM_CORRUPT,
            FS_ERR_NO_SPACE | FS_ERR_TABLE_FULL => PERM_NO_SPACE,
            _ => PERM_IO,
        });
    }
    Ok([result.status, result.value, CAP_NONE])
}

/// Scan the WHOLE fsd namespace, including unrecognized perm8-* names.
/// A short read, malformed latest record, gap, or duplicate poisons state;
/// no fallback to an older ALLOW. The only ignored files are outside perm8-*.
pub(super) fn scan(va: u64) -> Result<record::Recovery, u64> {
    let mut state = record::Scan::new();
    let mut cursor = 0u32;
    for _ in 0..=32 {
        let mut entry = [0u8; MSG_BYTES];
        let r = fs(FS_OP_LS, u64::from(cursor), CAP_NONE, &mut entry)?;
        let next = u32::from_le_bytes(entry[..4].try_into().unwrap());
        if next == FS_CURSOR_END {
            return state.finish().map_err(|_| PERM_CORRUPT);
        }
        let n = u32::from_le_bytes(entry[12..16].try_into().unwrap()) as usize;
        let size = u64::from_le_bytes(entry[4..12].try_into().unwrap());
        if next <= cursor
            || next > 32
            || n == 0
            || n >= FS_NAME_MAX
            || r[1] >= 32
            || u64::from(next) != r[1] + 1
        {
            return Err(PERM_CORRUPT);
        }
        let name = &entry[16..16 + n];
        if name.starts_with(b"perm8-") {
            if size == 0 {
                state.ingest(name, size, None).map_err(|_| PERM_CORRUPT)?;
            } else {
                if size != 512 {
                    return Err(PERM_CORRUPT);
                }
                let mut arg = [0; MSG_BYTES];
                arg[..n].copy_from_slice(name);
                let fh = fs(FS_OP_OPEN, 0, CAP_NONE, &mut arg)?[1];
                arg = [0; MSG_BYTES];
                arg[..8].copy_from_slice(&512u64.to_le_bytes());
                let read = fs(FS_OP_READ, fs_rw_w1(fh, 0), BUFFER_LENT, &mut arg);
                let mut close = [0; MSG_BYTES];
                let closed = fs(FS_OP_CLOSE, fh, CAP_NONE, &mut close);
                if read?.get(1) != Some(&512) || closed.is_err() {
                    return Err(PERM_IO);
                }
                let mut raw = [0u8; 512];
                unsafe { core::ptr::copy_nonoverlapping(va as *const u8, raw.as_mut_ptr(), 512) };
                state
                    .ingest(name, size, Some(&raw))
                    .map_err(|_| PERM_CORRUPT)?;
            }
        }
        cursor = next;
    }
    Err(PERM_CORRUPT)
}

pub(super) fn allowed(state: &record::Recovery) -> bool {
    state
        .current
        .is_some_and(|r| r.bytes() == record::decision(true))
}

/// On success returns the durable generation (no-op uses the existing one).
/// Once started is true, *any* subsequent error is ambiguous; caller must
/// latch DEGRADED and fail closed in RAM until broker restart/recovery.
pub(super) fn update(va: u64, requested: bool, started: &mut bool) -> Result<u64, u64> {
    let before = scan(va)?;
    let payload = record::decision(requested);
    if before.pending.is_none() && before.current.is_some_and(|r| r.bytes() == payload) {
        return Ok(u64::from(before.current.unwrap().sequence()));
    }
    let seq = before.next().ok_or(PERM_NO_SPACE)?;
    let mut raw = [0u8; 512];
    record::encode(seq, &payload, &mut raw).map_err(|_| PERM_BAD_INPUT)?;
    let name = record::name(seq).map_err(|_| PERM_BAD_INPUT)?;
    *started = true; // CREATE can commit even if the fsd reply is lost.
    let mut arg = [0; MSG_BYTES];
    arg[..name.len()].copy_from_slice(&name);
    let fh = if before.pending.is_none() {
        log_line(|o| o.str("permissiond: POLICY CREATE submitted"));
        let h = fs(FS_OP_CREATE, 0, CAP_NONE, &mut arg)?[1];
        log_line(|o| o.str("permissiond: POLICY CREATE committed empty generation"));
        h
    } else {
        log_line(|o| o.str("permissiond: POLICY resuming committed empty generation"));
        fs(FS_OP_OPEN, 0, CAP_NONE, &mut arg)?[1]
    };
    unsafe { core::ptr::copy_nonoverlapping(raw.as_ptr(), va as *mut u8, 512) };
    arg = [0; MSG_BYTES];
    arg[..8].copy_from_slice(&512u64.to_le_bytes());
    log_line(|o| o.str("permissiond: POLICY WRITE submitted"));
    let written = fs(FS_OP_WRITE, fs_rw_w1(fh, 0), BUFFER_LENT, &mut arg);
    if written.as_ref().is_ok_and(|r| r[1] == 512) {
        log_line(|o| o.str("permissiond: POLICY WRITE committed (fsd replied)"));
    }
    let mut close = [0; MSG_BYTES];
    let closed = fs(FS_OP_CLOSE, fh, CAP_NONE, &mut close);
    if written?.get(1) != Some(&512) || closed.is_err() {
        return Err(PERM_IO);
    }
    log_line(|o| o.str("permissiond: POLICY CLOSE completed"));
    let after = scan(va)?;
    if after.pending.is_some()
        || after
            .current
            .is_none_or(|r| r.sequence() != seq || r.bytes() != payload)
    {
        return Err(PERM_CORRUPT);
    }
    log_line(|o| o.str("permissiond: POLICY exact committed decision verified before reply"));
    Ok(u64::from(seq))
}
