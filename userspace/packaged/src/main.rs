#![no_std]
#![no_main]
//! ADR-0053 receiver: actual fsd cap + actual package endpoint + receiver-
//! verified disjoint marker. Verified bytes STAGE ONLY, never load an Image.
use arena_lib::fs::Client;
use arena_packaged::package::{self, Chain, Error, History, PACKAGE_MAX, VERIFY_SCRATCH};
use arena_phase84_crypto_audit::sha256;
use core::panic::PanicInfo;
#[path = "../../abi.rs"]
mod abi;
use abi::*;
#[path = "../../installed.rs"]
mod installed;

const FS: u64 = 0;
const SERVER: u64 = 1;
const MARKER: u64 = 2;
const REGISTRAR: u64 = 3;
const LIFECYCLE: u64 = 4;
const BUFFER: u64 = 7;
const LENT: u64 = 8;
// Private provisional Image cap: never an inherited grant or a public slot.
const PROVISIONAL: u64 = 9;

#[derive(Clone, Copy)]
struct Pending {
    token: u64,
    next_counter: u64,
    last_aborted: u64,
    id: [u8; 32],
    installed_hash: [u8; 32],
    package_digest: [u8; 32],
    policy_digest: [u8; 32],
    image_id: u32,
}
impl Pending {
    const fn empty() -> Self {
        Self {
            token: 0,
            next_counter: 1,
            last_aborted: 0,
            id: [0; 32],
            installed_hash: [0; 32],
            package_digest: [0; 32],
            policy_digest: [0; 32],
            image_id: 0,
        }
    }
    fn abort(&mut self) {
        if self.token == 0 {
            return;
        }
        // A failed revoke is a TCB invariant violation; leaving a runnable
        // provisional ID behind is not an acceptable typed refusal.
        if unsafe {
            syscall6(
                SYS_IMAGE_REVOKE,
                REGISTRAR,
                self.image_id as u64,
                0,
                0,
                0,
                0,
            )
        } != 0
            || unsafe { syscall1(SYS_CAP_DESTROY, PROVISIONAL) } != 0
        {
            fail("provisional Image revoke/drop refused");
        }
        self.token = 0;
        self.image_id = 0;
    }
}

fn register_payload(buf: &Buffers, size: usize) -> Result<u32, u64> {
    let n = size.checked_sub(192).ok_or(BAD_FORMAT)?;
    if !(1..=4096).contains(&n) {
        return Err(BAD_FORMAT);
    }
    let id = unsafe {
        syscall6(
            SYS_IMAGE_REGISTER,
            REGISTRAR,
            buf.file[128..].as_ptr() as u64,
            n as u64,
            PROVISIONAL,
            0,
            0,
        )
    };
    if id < 0 {
        return Err(if id == -4 { PKG_BUSY } else { DENY });
    }
    if !(27..=u32::MAX as i64).contains(&id) {
        fail("invalid minted Image ID");
    }
    let mut observed = [0u64; 3];
    if unsafe { syscall2(SYS_CAP_DESCRIBE, PROVISIONAL, observed.as_mut_ptr() as u64) } != 0
        || observed != [1, id as u64, RIGHTS_READ | RIGHTS_COPY | RIGHTS_DESTROY]
    {
        fail("minted Image cap inventory mismatch");
    }
    Ok(id as u32)
}
const BAD_FORMAT: u64 = PKG_BAD_FORMAT;
const DENY: u64 = PKG_DENY;
const NO_SPACE: u64 = PKG_NO_SPACE;
const DEGRADED: u64 = PKG_DEGRADED;
const CORRUPT: u64 = PKG_CORRUPT;
const COLLISION: u64 = PKG_COLLISION;
const OFFLINE: u64 = PKG_OFFLINE;
const OK: u64 = PKG_OK;
const ELIGIBLE: u64 = PKG_ELIGIBLE;
const UNSET: u64 = PKG_UNSET;
const INELIGIBLE: u64 = PKG_INELIGIBLE;
const PING_MAGIC: u64 = PKG_PING_MAGIC;

// The kernel enters with ONE 4-KiB stack page. Dalek's serial verifier
// alone uses >3 KiB and scan's nested frames can exceed that page. Switch
// before Rust executes to an image-owned, statically mapped 32-KiB .bss
// stack; no kernel primitive, no change to any other process's stack or ABI.
// The 4,288-byte artifact and 4,239-byte signature message live separately
// in .bss. 32 KiB is bounded and audited with -Z emit-stack-sizes.
#[repr(align(16))]
#[allow(dead_code)] // referenced from global_asm's symbol, not Rust field access
struct PrivateStack([u8; 32 * 1024]);
#[unsafe(no_mangle)]
static mut PACKAGE_STACK: PrivateStack = PrivateStack([0; 32 * 1024]);
core::arch::global_asm!(
    ".global _start",
    "_start:",
    "lea rsp, [rip + {stack} + 32768]",
    "and rsp, -16",
    "jmp {rust}",
    stack = sym PACKAGE_STACK,
    rust = sym start_on_private_stack,
);
struct Buffers {
    file: [u8; PACKAGE_MAX],
    signed: [u8; VERIFY_SCRATCH],
}
static mut BUFFERS: Buffers = Buffers {
    file: [0; PACKAGE_MAX],
    signed: [0; VERIFY_SCRATCH],
};

#[derive(Clone, Copy)]
struct Entry {
    name: [u8; 26],
    size: u64,
    seq: u8,
    policy: bool,
}
const EMPTY_ENTRY: Entry = Entry {
    name: [0; 26],
    size: 0,
    seq: 0,
    policy: false,
};
struct State {
    chain: Chain,
    history: History,
    id: Option<[u8; 32]>,
    objects: u8,
    input: Option<u64>,
    intent: Option<u64>,
}

fn say(s: &str) {
    log_line(|o| {
        o.str("packaged: ");
        o.str(s);
    });
}
fn fail(s: &str) -> ! {
    say(s);
    unsafe { syscall1(SYS_THREAD_EXIT, 89) };
    loop {
        core::hint::spin_loop();
    }
}
#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    fail("PANIC");
}
fn io_status(status: u64) -> u64 {
    match status {
        FS_ERR_NO_SPACE | FS_ERR_TABLE_FULL => NO_SPACE,
        FS_ERR_CORRUPT => CORRUPT,
        _ => OFFLINE,
    }
}
fn fs_reply(result: Result<arena_lib::ipc::Reply, arena_lib::fs::Error>) -> Result<u64, u64> {
    let r = result.map_err(|_| OFFLINE)?;
    if r.status != FS_OK {
        return Err(io_status(r.status));
    }
    Ok(r.value)
}
fn fs_name(name: &[u8], create: bool) -> Result<u64, u64> {
    let c = Client::new(FS);
    fs_reply(if create { c.create(name) } else { c.open(name) })
}
fn fs_close(handle: u64) -> Result<(), u64> {
    fs_reply(Client::new(FS).close(handle)).map(|_| ())
}

/// AFS1 block offset is sector-aligned; use 3584-byte then bounded remaining
/// chunks, never one 4,288-byte transfer or a nonaligned continuation.
fn read_named(name: &[u8], n: usize, va: u64, out: &mut [u8; PACKAGE_MAX]) -> Result<(), u64> {
    if n == 0 || n > PACKAGE_MAX {
        return Err(CORRUPT);
    }
    let fh = fs_name(name, false)?;
    let mut offset = 0usize;
    let mut failed = None;
    while offset < n {
        let amount = (n - offset).min(FS_XFER_MAX as usize);
        let got = fs_reply(Client::new(FS).read(fh, offset as u64, LENT, amount as u64));
        match got {
            Ok(v) if v == amount as u64 => {
                // frame is mapped, owned locally and LENT to fsd; copied only
                // after a checked exact read result.
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        va as *const u8,
                        out[offset..].as_mut_ptr(),
                        amount,
                    )
                };
                offset += amount;
            }
            Ok(_) => {
                failed = Some(CORRUPT);
                break;
            }
            Err(e) => {
                failed = Some(e);
                break;
            }
        }
    }
    let closed = fs_close(fh);
    if let Some(e) = failed {
        return Err(e);
    }
    closed
}
fn write_new(
    name: &[u8],
    n: usize,
    va: u64,
    file: &[u8; PACKAGE_MAX],
    degraded: &mut bool,
) -> Result<(), u64> {
    if n == 0 || n > PACKAGE_MAX {
        return Err(BAD_FORMAT);
    }
    // CREATE is the ambiguity boundary. Even a lost CREATE reply can have
    // committed an empty visible file; latch DEGRADED on any later failure.
    *degraded = true;
    let stage = name.starts_with(b"s8-");
    let install = name.starts_with(b"n8-");
    let decision = name.starts_with(b"v8-");
    say(if stage {
        "STAGE CREATE submitted"
    } else if install {
        "INSTALL CREATE submitted"
    } else if decision {
        "AACT CREATE submitted"
    } else {
        "POLICY CREATE submitted"
    });
    let fh = fs_name(name, true).map_err(|_| DEGRADED)?;
    say(if stage {
        "STAGE CREATE committed empty"
    } else if install {
        "INSTALL CREATE committed empty"
    } else if decision {
        "AACT CREATE committed empty"
    } else {
        "POLICY CREATE committed empty"
    });
    let mut offset = 0usize;
    let mut bad = false;
    while offset < n {
        let amount = (n - offset).min(FS_XFER_MAX as usize);
        unsafe { core::ptr::copy_nonoverlapping(file[offset..].as_ptr(), va as *mut u8, amount) };
        say(if stage {
            "STAGE WRITE submitted"
        } else if install {
            "INSTALL WRITE submitted"
        } else if decision {
            "AACT WRITE submitted"
        } else {
            "POLICY WRITE submitted"
        });
        match fs_reply(Client::new(FS).write(fh, offset as u64, LENT, amount as u64)) {
            Ok(got) if got == amount as u64 => {
                offset += amount;
                say(if stage {
                    "STAGE WRITE reply exact"
                } else if install {
                    "INSTALL WRITE reply exact"
                } else if decision {
                    "AACT WRITE reply exact"
                } else {
                    "POLICY WRITE reply exact"
                });
            }
            _ => {
                bad = true;
                break;
            }
        }
    }
    say(if stage {
        "STAGE CLOSE submitted"
    } else if install {
        "INSTALL CLOSE submitted"
    } else if decision {
        "AACT CLOSE submitted"
    } else {
        "POLICY CLOSE submitted"
    });
    let closed = fs_close(fh);
    if bad || closed.is_err() {
        return Err(DEGRADED);
    }
    say(if stage {
        "STAGE CLOSE completed"
    } else if install {
        "INSTALL CLOSE completed"
    } else if decision {
        "AACT CLOSE completed"
    } else {
        "POLICY CLOSE completed"
    });
    // Remain latched until caller rescans and verifies exact committed bytes.
    Ok(())
}
fn parse_status(e: Error) -> u64 {
    match e {
        Error::NoSpace => NO_SPACE,
        Error::Collision => COLLISION,
        Error::Corrupt => CORRUPT,
        Error::UnknownSigner | Error::Revoked | Error::Downgrade | Error::Conflict => DENY,
        Error::BadFormat | Error::BadSignature => BAD_FORMAT,
    }
}

fn scan(va: u64, buf: &mut Buffers, requested: &[u8; 32]) -> Result<State, u64> {
    let mut entries = [EMPTY_ENTRY; 6];
    let mut nentries = 0usize;
    let mut objects = 0u8;
    let mut seen_p = 0u8;
    let mut seen_s = 0u8;
    let mut prefix: Option<[u8; 20]> = None;
    let expected_prefix = package::namespace(requested).map_err(|_| BAD_FORMAT)?;
    let input_name = package::input_name(requested, false).map_err(|_| BAD_FORMAT)?;
    let intent_name = package::input_name(requested, true).map_err(|_| BAD_FORMAT)?;
    let (mut input, mut intent) = (None, None);
    let mut cursor = 0u32;
    for _ in 0..=32 {
        let mut wire = [0u8; MSG_BYTES];
        let result = fs_reply(Client::new(FS).list(cursor as u64, &mut wire))?;
        let next = u32::from_le_bytes(wire[..4].try_into().unwrap());
        if next == FS_CURSOR_END {
            break;
        }
        let size = u64::from_le_bytes(wire[4..12].try_into().unwrap());
        let n = u32::from_le_bytes(wire[12..16].try_into().unwrap()) as usize;
        if n == 0
            || n >= FS_NAME_MAX
            || next <= cursor
            || next > 32
            || result >= 32
            || u64::from(next) != result + 1
        {
            return Err(CORRUPT);
        }
        objects += 1;
        let name = &wire[16..16 + n];
        if name == input_name {
            if input.replace(size).is_some() {
                return Err(CORRUPT);
            }
        }
        if name == intent_name {
            if intent.replace(size).is_some() {
                return Err(CORRUPT);
            }
        }
        let policy = name.starts_with(b"p8-");
        let stage = name.starts_with(b"s8-");
        if policy || stage {
            if n != 26
                || name[23] != b'-'
                || name[24] != b'0'
                || !(b'1'..=if policy { b'4' } else { b'2' }).contains(&name[25])
                || name[3..23]
                    .iter()
                    .any(|b| !b.is_ascii_hexdigit() || b.is_ascii_uppercase())
            {
                return Err(CORRUPT);
            }
            let mut hash = [0; 20];
            hash.copy_from_slice(&name[3..23]);
            if let Some(prev) = prefix {
                if hash != prev {
                    return Err(COLLISION);
                }
            }
            prefix = Some(hash);
            if hash != expected_prefix {
                return Err(COLLISION);
            }
            let seq = name[25] - b'0';
            let mask = 1 << (seq - 1);
            let seen = if policy { &mut seen_p } else { &mut seen_s };
            if *seen & mask != 0 || nentries == entries.len() {
                return Err(CORRUPT);
            }
            *seen |= mask;
            let mut full = [0u8; 26];
            full.copy_from_slice(name);
            entries[nentries] = Entry {
                name: full,
                size,
                seq,
                policy,
            };
            nentries += 1;
        }
        cursor = next;
    }
    let policies = seen_p.count_ones() as u8;
    let stages = seen_s.count_ones() as u8;
    if seen_p != ((1u8 << policies) - 1) || seen_s != ((1u8 << stages) - 1) {
        return Err(CORRUPT);
    }
    let mut chain = Chain::new();
    let mut history = History::new();
    let mut id = None;
    for seq in 1..=policies {
        let entry = entries[..nentries]
            .iter()
            .find(|e| e.policy && e.seq == seq)
            .ok_or(CORRUPT)?;
        if entry.size != 512 {
            return Err(CORRUPT);
        }
        read_named(&entry.name, 512, va, &mut buf.file)?;
        let p = package::parse_policy(&buf.file[..512]).map_err(|_| CORRUPT)?;
        if p.id != *requested {
            return Err(COLLISION);
        }
        id = Some(p.id);
        chain.add(p).map_err(|_| CORRUPT)?;
    }
    for seq in 1..=stages {
        let entry = entries[..nentries]
            .iter()
            .find(|e| !e.policy && e.seq == seq)
            .ok_or(CORRUPT)?;
        let n = usize::try_from(entry.size).map_err(|_| CORRUPT)?;
        if !(193..=PACKAGE_MAX).contains(&n) {
            return Err(CORRUPT);
        }
        read_named(&entry.name, n, va, &mut buf.file)?;
        let pkg = package::parse_package(&buf.file[..n]).map_err(|_| CORRUPT)?;
        if pkg.id != *requested {
            return Err(COLLISION);
        }
        id = Some(pkg.id);
        history
            .ingest(&pkg, &chain, &mut buf.signed)
            .map_err(|_| CORRUPT)?;
    }
    Ok(State {
        chain,
        history,
        id,
        objects,
        input,
        intent,
    })
}

/// A complete same-platter lifecycle scan. Checks every visible newest record
/// and its predecessors, full IDs, exact signed staged bytes and immutable
/// SHA-256 links. Checksums are crash-prefix checks, not signatures.
struct Lifecycle {
    installs: [Option<(installed::Installed, installed::Digest)>; 2],
    n_ins: u8,
    latest: Option<(installed::Activation, installed::Digest)>,
    n_act: u8,
    selected_floor: Option<(u64, [u8; 32])>,
}
fn scan_lifecycle(
    va: u64,
    buf: &mut Buffers,
    requested: &[u8; 32],
    state: &State,
) -> Result<Lifecycle, u64> {
    let mut ins_names = [None::<[u8; 26]>; 2];
    let mut act_names = [None::<[u8; 26]>; 4];
    let mut cursor = 0u32;
    let expected_prefix = package::namespace(requested).map_err(|_| BAD_FORMAT)?;
    for _ in 0..=32 {
        let mut msg = [0u8; MSG_BYTES];
        let value = fs_reply(Client::new(FS).list(cursor as u64, &mut msg))?;
        let next = u32::from_le_bytes(msg[..4].try_into().unwrap());
        if next == FS_CURSOR_END {
            break;
        }
        let n = u32::from_le_bytes(msg[12..16].try_into().unwrap()) as usize;
        if n == 0
            || n >= FS_NAME_MAX
            || next <= cursor
            || next > 32
            || value >= 32
            || u64::from(next) != value + 1
        {
            return Err(CORRUPT);
        }
        let name = &msg[16..16 + n];
        let install = name.starts_with(b"n8-");
        let activation = name.starts_with(b"v8-");
        if install || activation {
            if n != 26
                || name[23] != b'-'
                || name[24] != b'0'
                || !(b'1'..=if install { b'2' } else { b'4' }).contains(&name[25])
                || name[3..23]
                    .iter()
                    .any(|b| !b.is_ascii_hexdigit() || b.is_ascii_uppercase())
                || u64::from_le_bytes(msg[4..12].try_into().unwrap()) != 512
            {
                return Err(CORRUPT);
            }
            if name[3..23] != expected_prefix {
                return Err(COLLISION);
            }
            let mut full = [0u8; 26];
            full.copy_from_slice(name);
            let idx = (name[25] - b'1') as usize;
            let target = if install {
                &mut ins_names[idx]
            } else {
                &mut act_names[idx]
            };
            if target.replace(full).is_some() {
                return Err(CORRUPT);
            }
        }
        cursor = next;
    }
    let mut result = Lifecycle {
        installs: [None; 2],
        n_ins: 0,
        latest: None,
        n_act: 0,
        selected_floor: None,
    };
    let mut prev = None::<[u8; 512]>;
    for (idx, name) in ins_names.into_iter().enumerate() {
        let Some(name) = name else {
            if ins_names[idx + 1..].iter().any(Option::is_some) {
                return Err(CORRUPT);
            }
            break;
        };
        read_named(&name, 512, va, &mut buf.file)?;
        let wire: [u8; 512] = buf.file[..512].try_into().unwrap();
        let rec = installed::parse_installed(&wire, sha256).map_err(|_| CORRUPT)?;
        if rec.id != *requested || rec.generation != (idx + 1) as u64 {
            return Err(CORRUPT);
        }
        installed::check_predecessor(
            requested,
            rec.generation,
            &rec.previous,
            prev.as_ref(),
            true,
            sha256,
        )
        .map_err(|_| CORRUPT)?;
        if rec.stage > state.history.count {
            return Err(CORRUPT);
        }
        let stage_name =
            package::numbered_name(b"s8-", requested, rec.stage, 2).map_err(|_| CORRUPT)?;
        let size = visible_size(&stage_name)?;
        if !(193..=PACKAGE_MAX as u64).contains(&size) {
            return Err(CORRUPT);
        }
        read_named(&stage_name, size as usize, va, &mut buf.file)?;
        let pkg = package::parse_package(&buf.file[..size as usize]).map_err(|_| CORRUPT)?;
        state
            .chain
            .verified_signer(&pkg, &mut buf.signed)
            .map_err(|_| CORRUPT)?;
        if pkg.id != *requested
            || pkg.version != rec.version
            || pkg.full_digest != rec.full_digest
            || buf.file[56..88] != rec.payload_digest
        {
            return Err(CORRUPT);
        }
        result.installs[idx] = Some((rec, sha256(&wire)));
        result.n_ins += 1;
        prev = Some(wire);
    }
    let mut prev = None::<[u8; 512]>;
    let mut active = false;
    let mut selected_floor = None::<(u64, [u8; 32])>;
    for (idx, name) in act_names.into_iter().enumerate() {
        let Some(name) = name else {
            if act_names[idx + 1..].iter().any(Option::is_some) {
                return Err(CORRUPT);
            }
            break;
        };
        read_named(&name, 512, va, &mut buf.file)?;
        let wire: [u8; 512] = buf.file[..512].try_into().unwrap();
        let rec = installed::parse_activation(&wire, sha256).map_err(|_| CORRUPT)?;
        if rec.id != *requested || rec.generation != (idx + 1) as u64 {
            return Err(CORRUPT);
        }
        installed::check_predecessor(
            requested,
            rec.generation,
            &rec.previous,
            prev.as_ref(),
            false,
            sha256,
        )
        .map_err(|_| CORRUPT)?;
        if rec.select {
            if !result.installs.iter().flatten().any(|(i, hash)| {
                *hash == rec.installed_hash
                    && i.full_digest == rec.full_digest
                    && i.version == rec.version
            }) {
                return Err(CORRUPT);
            }
            if let Some((version, digest)) = selected_floor {
                if rec.version < version || (rec.version == version && rec.full_digest != digest) {
                    return Err(CORRUPT);
                }
            }
            selected_floor = Some((rec.version, rec.full_digest));
            active = true;
        } else {
            if !active {
                return Err(CORRUPT);
            }
            active = false;
        }
        result.latest = Some((rec, sha256(&wire)));
        result.selected_floor = selected_floor;
        result.n_act += 1;
        prev = Some(wire);
    }
    Ok(result)
}

/// The exact signed current policy file, not its ordinal or a cached flag,
/// binds a volatile PREPARE across any subsequent request.
fn current_policy_hash(
    state: &State,
    requested: &[u8; 32],
    va: u64,
    buf: &mut Buffers,
) -> Result<[u8; 32], u64> {
    if state.chain.count == 0 {
        return Ok([0; 32]);
    }
    let name =
        package::numbered_name(b"p8-", requested, state.chain.count, 4).map_err(|_| CORRUPT)?;
    read_named(&name, 512, va, &mut buf.file)?;
    Ok(sha256(&buf.file[..512]))
}

/// Leave the exact independently verified signed staged bytes in buf.file for
/// the kernel's single-copy registration. Never infer eligibility from QUERY.
fn verified_install(
    rec: &installed::Installed,
    state: &State,
    requested: &[u8; 32],
    va: u64,
    buf: &mut Buffers,
) -> Result<usize, u64> {
    let name = package::numbered_name(b"s8-", requested, rec.stage, 2).map_err(|_| CORRUPT)?;
    let size = visible_size(&name)?;
    if !(193..=PACKAGE_MAX as u64).contains(&size) {
        return Err(CORRUPT);
    }
    read_named(&name, size as usize, va, &mut buf.file)?;
    let pkg = package::parse_package(&buf.file[..size as usize]).map_err(|_| CORRUPT)?;
    if pkg.id != *requested
        || pkg.full_digest != rec.full_digest
        || pkg.version != rec.version
        || buf.file[56..88] != rec.payload_digest
    {
        return Err(CORRUPT);
    }
    state
        .chain
        .eligible(&pkg, &mut buf.signed)
        .map_err(|_| DENY)?;
    Ok(size as usize)
}

/// Return (typed status, word1); an Image may only be replied to a fresh
/// marker-approved COMMIT/LAUNCH after exact disk/signer/authority checks.
fn dispatch(
    op: u64,
    arg: u64,
    requested: &[u8; 32],
    expected_digest: &[u8; 32],
    va: u64,
    buf: &mut Buffers,
    degraded: &mut bool,
    answer: &mut [u8; MSG_BYTES],
    pending: &mut Pending,
    reply_cap: &mut u64,
) -> (u64, u64) {
    if *degraded {
        pending.abort();
        return (DEGRADED, 0);
    }
    let state = match scan(va, buf, requested) {
        Ok(s) => s,
        Err(e) => {
            pending.abort();
            return (e, 0);
        }
    };
    if state.id.is_some_and(|id| id != *requested) {
        pending.abort();
        return (COLLISION, 0);
    }
    let lifecycle = match scan_lifecycle(va, buf, requested, &state) {
        Ok(s) => s,
        Err(e) => {
            pending.abort();
            return (e, 0);
        }
    };
    // Every request rechecks a live provisional binding, including ordinary
    // QUERY; a modified staged file or signed policy cannot retain an old ID.
    if pending.token != 0 {
        let bound = lifecycle
            .installs
            .iter()
            .flatten()
            .find(|(_, h)| *h == pending.installed_hash)
            .copied();
        let unchanged = if let Some((rec, _)) = bound {
            current_policy_hash(&state, requested, va, buf).ok() == Some(pending.policy_digest)
                && verified_install(&rec, &state, requested, va, buf).is_ok()
                && rec.full_digest == pending.package_digest
                && rec.id == pending.id
        } else {
            false
        };
        if !unchanged {
            pending.abort();
        }
    }
    // No policy/STAGE mutation may leave an old provisional Image live.
    if matches!(
        op,
        PKG_OP_POLICY | PKG_OP_STAGE | PKG_OP_INSTALL | PKG_OP_DEACTIVATE
    ) {
        pending.abort();
    }
    if op == PKG_OP_INSTALL {
        if !(1..=2).contains(&arg) || state.history.count < arg as u8 {
            return (BAD_FORMAT, 0);
        }
        let stage_name = package::numbered_name(b"s8-", requested, arg as u8, 2).unwrap();
        let size = match visible_size(&stage_name) {
            Ok(n) if (193..=PACKAGE_MAX as u64).contains(&n) => n as usize,
            Ok(_) => return (CORRUPT, 0),
            Err(e) => return (e, 0),
        };
        if let Err(e) = read_named(&stage_name, size, va, &mut buf.file) {
            return (e, 0);
        }
        let pkg = match package::parse_package(&buf.file[..size]) {
            Ok(p) => p,
            Err(_) => return (CORRUPT, 0),
        };
        if pkg.id != *requested || pkg.full_digest != *expected_digest {
            return (PKG_STALE, 0);
        }
        if state.chain.eligible(&pkg, &mut buf.signed).is_err() {
            return (DENY, 0);
        }
        let version = pkg.version;
        let digest = pkg.full_digest;
        let mut payload = [0u8; 32];
        payload.copy_from_slice(&buf.file[56..88]);
        if lifecycle.n_ins > 0 {
            let (prev, prev_hash) = lifecycle.installs[(lifecycle.n_ins - 1) as usize].unwrap();
            if version < prev.version {
                return (PKG_DOWNGRADE, 0);
            }
            if version == prev.version {
                if digest != prev.full_digest {
                    return (PKG_CONFLICT, 0);
                }
                if prev.stage != arg as u8 {
                    return (PKG_CONFLICT, 0);
                }
                answer[..32].copy_from_slice(&digest);
                answer[32..64].copy_from_slice(&prev_hash);
                return (PKG_INSTALLED, prev.generation);
            }
        }
        if lifecycle.n_ins >= 2 || state.objects >= 32 {
            return (NO_SPACE, 0);
        }
        let generation = lifecycle.n_ins + 1;
        let prev = lifecycle.installs[0].map_or([0; 32], |(_, h)| h);
        let record = installed::Installed {
            generation: generation as u64,
            id: *requested,
            full_digest: digest,
            version,
            payload_digest: payload,
            stage: arg as u8,
            observed_policy: state.chain.count,
            previous: prev,
        };
        let mut wire = [0u8; 512];
        if installed::encode_installed(&record, &mut wire, sha256).is_err() {
            return (CORRUPT, 0);
        }
        let record_hash = sha256(&wire);
        let name = package::numbered_name(b"n8-", requested, generation, 2).unwrap();
        buf.file[..512].copy_from_slice(&wire);
        if write_new(&name, 512, va, &buf.file, degraded).is_err() {
            return (DEGRADED, 0);
        }
        let after = match scan(va, buf, requested) {
            Ok(s) => s,
            Err(_) => return (DEGRADED, 0),
        };
        let history = match scan_lifecycle(va, buf, requested, &after) {
            Ok(s) => s,
            Err(_) => return (DEGRADED, 0),
        };
        if history.n_ins != generation
            || history.installs[(generation - 1) as usize] != Some((record, record_hash))
            || read_named(&name, 512, va, &mut buf.file).is_err()
            || buf.file[..512] != wire
        {
            return (DEGRADED, 0);
        }
        *degraded = false;
        answer[..32].copy_from_slice(&digest);
        answer[32..64].copy_from_slice(&record_hash);
        return (PKG_INSTALLED, generation as u64);
    }
    if op == PKG_OP_SELECT_PREPARE {
        if !(1..=2).contains(&arg) {
            return (BAD_FORMAT, 0);
        }
        let Some((rec, hash)) = lifecycle.installs[(arg - 1) as usize] else {
            return (PKG_STALE, 0);
        };
        if hash != *expected_digest {
            return (PKG_STALE, 0);
        }
        let policy = match current_policy_hash(&state, requested, va, buf) {
            Ok(p) => p,
            Err(e) => {
                pending.abort();
                return (e, 0);
            }
        };
        let size = match verified_install(&rec, &state, requested, va, buf) {
            Ok(n) => n,
            Err(e) => {
                pending.abort();
                return (e, 0);
            }
        };
        if let Some((version, digest)) = lifecycle.selected_floor {
            if rec.version < version {
                return (PKG_DOWNGRADE, 0);
            }
            if rec.version == version && rec.full_digest != digest {
                return (PKG_CONFLICT, 0);
            }
        }
        if let Some((latest, _)) = lifecycle.latest {
            if latest.select && latest.installed_hash == hash {
                pending.abort();
                answer[..32].copy_from_slice(&rec.full_digest);
                answer[32..].copy_from_slice(&hash);
                return (PKG_ACTIVE, latest.generation);
            }
        }
        if pending.token != 0 {
            if pending.id != *requested
                || pending.installed_hash != hash
                || pending.package_digest != rec.full_digest
                || pending.policy_digest != policy
                || pending.token as u8 != lifecycle.n_act + 1
            {
                return (PKG_BUSY, 0);
            }
            answer[..32].copy_from_slice(&rec.full_digest);
            answer[32..].copy_from_slice(&hash);
            return (PKG_PREPARED, pending.token);
        }
        if lifecycle.n_act >= 4 || state.objects >= 32 || pending.next_counter > (u64::MAX >> 8) {
            return (NO_SPACE, 0);
        }
        let image_id = match register_payload(buf, size) {
            Ok(id) => id,
            Err(e) => return (e, 0),
        };
        let token = (pending.next_counter << 8) | u64::from(lifecycle.n_act + 1);
        pending.next_counter += 1;
        pending.token = token;
        pending.id = *requested;
        pending.installed_hash = hash;
        pending.package_digest = rec.full_digest;
        pending.policy_digest = policy;
        pending.image_id = image_id;
        answer[..32].copy_from_slice(&rec.full_digest);
        answer[32..].copy_from_slice(&hash);
        return (PKG_PREPARED, token);
    }
    if op == PKG_OP_ABORT {
        if arg == 0 || arg >> 8 == 0 || expected_digest == &[0; 32] {
            return (BAD_FORMAT, 0);
        }
        if pending.token == arg
            && pending.id == *requested
            && pending.installed_hash == *expected_digest
        {
            pending.abort();
            pending.last_aborted = arg;
            return (OK, 0);
        }
        if pending.last_aborted == arg
            && pending.id == *requested
            && pending.installed_hash == *expected_digest
        {
            return (OK, 0);
        }
        return (PKG_STALE, 0);
    }
    if op == PKG_OP_SELECT_COMMIT {
        let generation = (arg & 255) as u8;
        if arg >> 8 == 0 || !(1..=4).contains(&generation) {
            return (BAD_FORMAT, 0);
        }
        let Some((rec, hash)) = lifecycle
            .installs
            .iter()
            .flatten()
            .find(|(_, h)| h == expected_digest)
            .copied()
        else {
            pending.abort();
            return (PKG_STALE, 0);
        };
        let policy = match current_policy_hash(&state, requested, va, buf) {
            Ok(p) => p,
            Err(e) => {
                pending.abort();
                return (e, 0);
            }
        };
        let _size = match verified_install(&rec, &state, requested, va, buf) {
            Ok(n) => n,
            Err(e) => {
                pending.abort();
                return (e, 0);
            }
        };
        // A lost reply or verifier restart cannot fabricate a new commit.
        if let Some((latest, latest_hash)) = lifecycle.latest {
            if latest.select
                && latest.generation == generation as u64
                && latest.installed_hash == hash
                && latest.full_digest == rec.full_digest
            {
                pending.abort();
                answer[..32].copy_from_slice(&rec.full_digest);
                answer[32..].copy_from_slice(&latest_hash);
                return (PKG_ACTIVE, latest.generation);
            }
        }
        if pending.token != arg
            || pending.id != *requested
            || pending.installed_hash != hash
            || pending.package_digest != rec.full_digest
            || pending.policy_digest != policy
            || generation != lifecycle.n_act + 1
        {
            pending.abort();
            return (PKG_STALE, 0);
        }
        if lifecycle.n_act >= 4 || state.objects >= 32 {
            pending.abort();
            return (NO_SPACE, 0);
        }
        let previous = lifecycle.latest.map_or([0; 32], |(_, h)| h);
        let selected = installed::Activation {
            generation: generation as u64,
            id: *requested,
            installed_hash: hash,
            full_digest: rec.full_digest,
            version: rec.version,
            select: true,
            previous,
        };
        let mut wire = [0u8; 512];
        if installed::encode_activation(&selected, &mut wire, sha256).is_err() {
            pending.abort();
            return (CORRUPT, 0);
        }
        let record_hash = sha256(&wire);
        let name = package::numbered_name(b"v8-", requested, generation, 4).unwrap();
        buf.file[..512].copy_from_slice(&wire);
        if write_new(&name, 512, va, &buf.file, degraded).is_err() {
            pending.abort();
            return (DEGRADED, 0);
        }
        let exact = scan(va, buf, requested).and_then(|s| scan_lifecycle(va, buf, requested, &s));
        if !matches!(exact, Ok(l) if l.latest == Some((selected, record_hash)))
            || read_named(&name, 512, va, &mut buf.file).is_err()
            || buf.file[..512] != wire
        {
            pending.abort();
            return (DEGRADED, 0);
        }
        *degraded = false;
        answer[..32].copy_from_slice(&rec.full_digest);
        answer[32..].copy_from_slice(&record_hash);
        *reply_cap = PROVISIONAL;
        pending.token = 0; // keep the cap through IPC_REPLY, destroy source afterwards
        return (
            PKG_ACTIVE,
            (u64::from(pending.image_id) << 8) | generation as u64,
        );
    }
    if op == PKG_OP_DEACTIVATE {
        if !(1..=4).contains(&arg) {
            return (BAD_FORMAT, 0);
        }
        let Some((last, hash)) = lifecycle.latest else {
            return (PKG_STALE, 0);
        };
        if !last.select && arg == last.generation && last.previous == *expected_digest {
            answer[..32].copy_from_slice(&hash);
            return (PKG_DEACTIVATED, arg);
        }
        if !last.select || arg != last.generation + 1 || *expected_digest != hash {
            return (PKG_STALE, 0);
        }
        if state.objects >= 32 {
            return (NO_SPACE, 0);
        }
        let disabled = installed::Activation {
            generation: arg,
            id: *requested,
            installed_hash: [0; 32],
            full_digest: [0; 32],
            version: 0,
            select: false,
            previous: hash,
        };
        let mut wire = [0u8; 512];
        if installed::encode_activation(&disabled, &mut wire, sha256).is_err() {
            return (CORRUPT, 0);
        }
        let record_hash = sha256(&wire);
        let name = package::numbered_name(b"v8-", requested, arg as u8, 4).unwrap();
        buf.file[..512].copy_from_slice(&wire);
        if write_new(&name, 512, va, &buf.file, degraded).is_err() {
            return (DEGRADED, 0);
        }
        let exact = scan(va, buf, requested).and_then(|s| scan_lifecycle(va, buf, requested, &s));
        if !matches!(exact, Ok(l) if l.latest == Some((disabled, record_hash)))
            || read_named(&name, 512, va, &mut buf.file).is_err()
            || buf.file[..512] != wire
        {
            return (DEGRADED, 0);
        }
        *degraded = false;
        answer[..32].copy_from_slice(&record_hash);
        return (PKG_DEACTIVATED, arg);
    }
    if op == PKG_OP_LAUNCH {
        if !(1..=4).contains(&arg) {
            return (BAD_FORMAT, 0);
        }
        let Some((selected, hash)) = lifecycle.latest else {
            return (PKG_STALE, 0);
        };
        if !selected.select
            || selected.generation != arg
            || hash != *expected_digest
            || pending.token != 0
        {
            return (PKG_STALE, 0);
        }
        let Some((rec, _)) = lifecycle
            .installs
            .iter()
            .flatten()
            .find(|(_, h)| *h == selected.installed_hash)
            .copied()
        else {
            return (CORRUPT, 0);
        };
        let size = match verified_install(&rec, &state, requested, va, buf) {
            Ok(n) => n,
            Err(e) => return (e, 0),
        };
        let image_id = match register_payload(buf, size) {
            Ok(id) => id,
            Err(e) => return (e, 0),
        };
        answer[..32].copy_from_slice(&rec.full_digest);
        answer[32..].copy_from_slice(&hash);
        *reply_cap = PROVISIONAL;
        return (PKG_LAUNCH_READY, (u64::from(image_id) << 8) | arg);
    }
    if op == PKG_OP_QUERY {
        // QUERY never mutates disk or authorizes a stage
        if state.history.count == 0 {
            return (UNSET, 0);
        }
        let name = package::numbered_name(b"s8-", requested, state.history.count, 2).unwrap();
        // Re-lookup size via LS rather than infer from version or filename.
        let size = match visible_size(&name) {
            Ok(n) => n,
            Err(e) => return (e, 0),
        };
        if !(193..=PACKAGE_MAX as u64).contains(&size) {
            return (CORRUPT, 0);
        }
        if let Err(e) = read_named(&name, size as usize, va, &mut buf.file) {
            return (e, 0);
        }
        let pkg = match package::parse_package(&buf.file[..size as usize]) {
            Ok(p) => p,
            Err(_) => return (CORRUPT, 0),
        };
        return match state.history.decision(&pkg, &state.chain, &mut buf.signed) {
            Ok(()) => {
                answer[..32].copy_from_slice(&pkg.full_digest);
                (ELIGIBLE, pkg.version)
            }
            Err(Error::Revoked | Error::Downgrade) => (INELIGIBLE, 0),
            Err(e) => (parse_status(e), 0),
        };
    }
    if op == PKG_OP_POLICY {
        // A Power shell still holds the frozen 8.4 STAGE/POLICY marker, not
        // the 8.5 manager's Process caps. It cannot revoke a running child
        // or every copied Image ID. Refuse *before CREATE* while a durable
        // select exists: the manager must first freeze launches, revoke its
        // ID, stop/reap the child and commit DEACTIVATE using its distinct
        // lifecycle marker. This preserves v1 op/status bytes (DENY=-2)
        // without ever acknowledging an unsafe policy revocation.
        if lifecycle
            .latest
            .is_some_and(|(decision, _)| decision.select)
        {
            return (DENY, 0);
        }
        // POLICY: offline-root-signed intent, no local signing.
        if state.chain.count >= package::MAX_POLICIES {
            return (NO_SPACE, 0);
        }
        if state.objects >= 32 {
            return (NO_SPACE, 0);
        }
        if state.intent != Some(512) {
            return (BAD_FORMAT, 0);
        }
        if let Err(e) = read_named(
            &package::input_name(requested, true).unwrap(),
            512,
            va,
            &mut buf.file,
        ) {
            return (e, 0);
        }
        let next = match package::parse_policy(&buf.file[..512]) {
            Ok(p) => p,
            Err(e) => return (parse_status(e), 0),
        };
        if next.id != *requested {
            return (COLLISION, 0);
        }
        if next.generation != state.chain.count + 1 {
            return (BAD_FORMAT, 0);
        }
        let mut proposed = state.chain;
        if let Err(e) = proposed.add(next) {
            return (parse_status(e), 0);
        }
        let signed_digest = sha256(&buf.file[..512]);
        let name = package::numbered_name(b"p8-", requested, next.generation, 4).unwrap();
        if write_new(&name, 512, va, &buf.file, degraded).is_err() {
            return (DEGRADED, 0);
        }
        let after = match scan(va, buf, requested) {
            Ok(s) => s,
            Err(_) => return (DEGRADED, 0),
        };
        if after.chain.count != next.generation || after.chain.current != Some(next) {
            return (DEGRADED, 0);
        }
        // Semantic equality alone does not authenticate the exact persisted
        // signed record: explicitly reread the newly named immutable file.
        if read_named(&name, 512, va, &mut buf.file).is_err()
            || sha256(&buf.file[..512]) != signed_digest
        {
            return (DEGRADED, 0);
        }
        *degraded = false;
        say("POLICY exact persisted bytes verified before acknowledgement");
        return (OK, next.generation as u64);
    }
    if op == PKG_OP_STAGE {
        // STAGE: never installs, activates or grants Image authority.
        let size = match state.input {
            Some(n) if (193..=PACKAGE_MAX as u64).contains(&n) => n as usize,
            _ => return (BAD_FORMAT, 0),
        };
        if let Err(e) = read_named(
            &package::input_name(requested, false).unwrap(),
            size,
            va,
            &mut buf.file,
        ) {
            return (e, 0);
        }
        let pkg = match package::parse_package(&buf.file[..size]) {
            Ok(p) => p,
            Err(e) => return (parse_status(e), 0),
        };
        if pkg.id != *requested {
            return (COLLISION, 0);
        }
        let needed = match state
            .history
            .stage_candidate(&pkg, &state.chain, &mut buf.signed)
        {
            Ok(value) => value,
            Err(e) => return (parse_status(e), 0),
        };
        let digest = pkg.full_digest;
        let version = pkg.version;
        if !needed {
            answer[..32].copy_from_slice(&digest);
            return (ELIGIBLE, version);
        }
        if state.objects >= 32 {
            return (NO_SPACE, 0);
        }
        let name = package::numbered_name(b"s8-", requested, state.history.count + 1, 2).unwrap();
        if write_new(&name, size, va, &buf.file, degraded).is_err() {
            return (DEGRADED, 0);
        }
        let after = match scan(va, buf, requested) {
            Ok(s) => s,
            Err(_) => return (DEGRADED, 0),
        };
        if after.history.latest
            != Some(package::Stage {
                id: *requested,
                version,
                full_digest: digest,
            })
        {
            return (DEGRADED, 0);
        }
        let size = match visible_size(&name) {
            Ok(n) => n as usize,
            Err(_) => return (DEGRADED, 0),
        };
        if let Err(_) = read_named(&name, size, va, &mut buf.file) {
            return (DEGRADED, 0);
        }
        let pkg = match package::parse_package(&buf.file[..size]) {
            Ok(p) => p,
            Err(_) => return (DEGRADED, 0),
        };
        if after
            .history
            .decision(&pkg, &after.chain, &mut buf.signed)
            .is_err()
        {
            return (DEGRADED, 0);
        }
        answer[..32].copy_from_slice(&digest);
        *degraded = false;
        say("STAGE exact persisted bytes verified before acknowledgement");
        return (ELIGIBLE, version);
    }
    (BAD_FORMAT, 0)
}

/// Checked bounded LS lookup for the reread; never guess a persisted size.
fn visible_size(name: &[u8]) -> Result<u64, u64> {
    let mut cursor = 0u32;
    for _ in 0..=32 {
        let mut msg = [0; MSG_BYTES];
        let value = fs_reply(Client::new(FS).list(cursor as u64, &mut msg))?;
        let next = u32::from_le_bytes(msg[..4].try_into().unwrap());
        if next == FS_CURSOR_END {
            return Err(CORRUPT);
        }
        let len = u32::from_le_bytes(msg[12..16].try_into().unwrap()) as usize;
        if len == 0
            || len >= FS_NAME_MAX
            || next <= cursor
            || next > 32
            || value >= 32
            || u64::from(next) != value + 1
        {
            return Err(CORRUPT);
        }
        if &msg[16..16 + len] == name {
            return Ok(u64::from_le_bytes(msg[4..12].try_into().unwrap()));
        }
        cursor = next;
    }
    Err(CORRUPT)
}

/// Readiness never hides a malformed visible signed namespace behind a
/// syntactically valid PING. Select ID from a visible record, then perform
/// the same full ordered scan used on every request. Names alone never
/// authorize it: `scan` verifies the root/signature and full signed ID.
fn boot_scan(va: u64, buf: &mut Buffers) -> Result<(), u64> {
    let mut cursor = 0u32;
    let mut id = None;
    for _ in 0..=32 {
        let mut msg = [0u8; MSG_BYTES];
        let value = fs_reply(Client::new(FS).list(cursor as u64, &mut msg))?;
        let next = u32::from_le_bytes(msg[..4].try_into().unwrap());
        if next == FS_CURSOR_END {
            break;
        }
        let size = u64::from_le_bytes(msg[4..12].try_into().unwrap());
        let n = u32::from_le_bytes(msg[12..16].try_into().unwrap()) as usize;
        if n == 0
            || n >= FS_NAME_MAX
            || next <= cursor
            || next > 32
            || value >= 32
            || u64::from(next) != value + 1
        {
            return Err(CORRUPT);
        }
        let name = &msg[16..16 + n];
        let pol = name.starts_with(b"p8-");
        let stage = name.starts_with(b"s8-");
        let installed_record = name.starts_with(b"n8-");
        let decision_record = name.starts_with(b"v8-");
        if pol || stage || installed_record || decision_record {
            if n != 26 || size > PACKAGE_MAX as u64 || size == 0 {
                return Err(CORRUPT);
            }
            read_named(name, size as usize, va, &mut buf.file)?;
            let full = &buf.file[..size as usize];
            let observed_id = if pol {
                package::parse_policy(full).map_err(|_| CORRUPT)?.id
            } else if stage {
                package::parse_package(full).map_err(|_| CORRUPT)?.id
            } else if installed_record {
                installed::parse_installed(full, sha256)
                    .map_err(|_| CORRUPT)?
                    .id
            } else {
                installed::parse_activation(full, sha256)
                    .map_err(|_| CORRUPT)?
                    .id
            };
            if id.is_some_and(|prior| prior != observed_id) {
                return Err(COLLISION);
            }
            id = Some(observed_id);
        }
        cursor = next;
    }
    if let Some(id) = id {
        let state = scan(va, buf, &id)?;
        scan_lifecycle(va, buf, &id, &state)?;
    }
    Ok(())
}

#[unsafe(no_mangle)]
pub extern "C" fn start_on_private_stack() -> ! {
    let mut actual = [[0u64; 3]; 5];
    for (slot, cap) in actual.iter_mut().enumerate() {
        if unsafe { syscall2(SYS_CAP_DESCRIBE, slot as u64, cap.as_mut_ptr() as u64) } != 0 {
            fail("bootstrap cap missing");
        }
    }
    if actual[0][0] != 2
        || actual[0][2] != RIGHTS_WRITE
        || actual[1][0] != 2
        || actual[1][2] != RIGHTS_READ
        || actual[2][0] != 3
        || actual[2][2] != RIGHTS_READ
        || actual[REGISTRAR as usize] != [5, 0, RIGHTS_WRITE]
        || actual[LIFECYCLE as usize][0] != 3
        || actual[LIFECYCLE as usize][2] != RIGHTS_READ
        || actual[LIFECYCLE as usize][1] == actual[MARKER as usize][1]
        || actual[0][1] == actual[1][1]
    {
        fail("bootstrap authority differs from literal five grants");
    }
    let mut extra = [0u64; 3];
    for slot in 5..32 {
        if unsafe { syscall2(SYS_CAP_DESCRIBE, slot, extra.as_mut_ptr() as u64) } == 0 {
            fail("unexpected bootstrap authority");
        }
    }
    if unsafe { syscall1(SYS_ALLOC_FRAME, BUFFER) } <= 0
        || unsafe { syscall3(SYS_CAP_COPY, BUFFER, LENT, RIGHTS_ALL) } != 0
    {
        fail("DMA frame allocation refused");
    }
    let va = unsafe { syscall2(SYS_MAP_MEMORY, BUFFER, 1) };
    if va <= 0 {
        fail("DMA frame map refused");
    }
    // Single-threaded IPC_RECV serializes every borrow of the .bss buffers.
    let buf = unsafe { &mut *core::ptr::addr_of_mut!(BUFFERS) };
    if boot_scan(va as u64, buf).is_err() {
        fail("boot namespace scan refused: corrupt/offline, no READY");
    }
    say("boot with exact FS/W endpoint/R STAGE/R registrar/W lifecycle/R; namespace scan verified");
    let mut degraded = false;
    let mut pending = Pending::empty();
    loop {
        let mut words = [0u64; 3];
        let mut req = [0u8; MSG_BYTES];
        if unsafe {
            syscall3(
                SYS_IPC_RECV,
                SERVER,
                words.as_mut_ptr() as u64,
                req.as_mut_ptr() as u64,
            )
        } < 0
        {
            fail("recv refused");
        }
        let (op, arg, landed) = (words[0], words[1], words[2]);
        // Consume every landed reference, even on PING, malformed requests,
        // wrong-kind/rights/marker or backend failure. A numeric slot is never
        // approval. `take_diagnostic` compares Notification object identity
        // and distinct anchor/sent literal rights before SYS_CAP_DESTROY.
        let marked = if landed == CAP_NONE {
            false
        } else {
            take_diagnostic(
                landed,
                if (PKG_OP_INSTALL..=PKG_OP_ABORT).contains(&op) {
                    LIFECYCLE
                } else {
                    MARKER
                },
            )
        };
        let mut reply = [0u8; MSG_BYTES];
        let mut reply_cap = CAP_NONE;
        let (status, value) = if op <= PKG_OP_STAGE
            && (arg != 0 || req[32..].iter().any(|&b| b != 0))
        {
            (BAD_FORMAT, 0)
        } else if op == PKG_OP_PING {
            if landed != CAP_NONE || req != [0; MSG_BYTES] {
                (BAD_FORMAT, 0)
            } else {
                (OK, PING_MAGIC)
            }
        } else if !(PKG_OP_QUERY..=PKG_OP_ABORT).contains(&op) || !package::canonical_id(&req[..32])
        {
            (BAD_FORMAT, 0)
        } else if op == PKG_OP_QUERY && landed != CAP_NONE {
            (BAD_FORMAT, 0)
        } else if op != PKG_OP_QUERY && !marked {
            (DENY, 0)
        } else {
            let mut id = [0u8; 32];
            id.copy_from_slice(&req[..32]);
            let mut digest = [0u8; 32];
            digest.copy_from_slice(&req[32..64]);
            dispatch(
                op,
                arg,
                &id,
                &digest,
                va as u64,
                buf,
                &mut degraded,
                &mut reply,
                &mut pending,
                &mut reply_cap,
            )
        };
        // Exact post-dispatch occupancy: 0..4 bootstrap, slot 8 local
        // LENT frame; slot 7 was consumed by self-map. A wrong-kind or
        // attenuated landed cap may lack DESTROY but IPC provenance still
        // permits disposal. Never leave it to exhaust the 32-slot table.
        for slot in 5..32 {
            if slot == LENT
                || (slot == PROVISIONAL && (pending.token != 0 || reply_cap == PROVISIONAL))
            {
                continue;
            }
            let mut found = [0u64; 3];
            if unsafe { syscall2(SYS_CAP_DESCRIBE, slot, found.as_mut_ptr() as u64) } == 0 {
                fail("post-dispatch cap occupancy leaked");
            }
        }
        if unsafe {
            syscall5(
                SYS_IPC_REPLY,
                SERVER,
                status,
                value,
                reply_cap,
                reply.as_ptr() as u64,
            )
        } < 0
        {
            if reply_cap == PROVISIONAL {
                let mut described = [0u64; 3];
                if unsafe { syscall2(SYS_CAP_DESCRIBE, PROVISIONAL, described.as_mut_ptr() as u64) }
                    == 0
                {
                    let _ =
                        unsafe { syscall6(SYS_IMAGE_REVOKE, REGISTRAR, described[1], 0, 0, 0, 0) };
                    let _ = unsafe { syscall1(SYS_CAP_DESTROY, PROVISIONAL) };
                }
            }
            fail("reply refused");
        }
        // IPC_REPLY snapshots the source; deleting it does NOT revoke the
        // receiver's landed reference. No provisional Image survives COMMIT
        // or LAUNCH in the verifier after a successful transfer.
        if reply_cap == PROVISIONAL && unsafe { syscall1(SYS_CAP_DESTROY, PROVISIONAL) } != 0 {
            fail("transferred Image source destroy refused");
        }
    }
}
