//! ArenaOS minimal shell — the first input-driven program (M4.6,
//! ADR-0020; filesystem builtins M5.3, ADR-0023).
//!
//! A genuine cargo/rust-lld ELF64 artifact (`x86_64-unknown-none`,
//! `shell.ld`) at its own fixed window: `0x400000` text (R+X),
//! `0x410000` data+bss (R+W, NOLOAD bss). It is embedded into the
//! kernel as spawn-registry image 1 and spawned at boot as the initial
//! service through `spawn::spawn_init` — the M4.5 creation sequence
//! with kernel-literal grants:
//!
//! - slot 0: `Power` (WRITE) — the authority behind `shutdown`,
//! - slot 1: `Image{0}` (READ) — the M4.3 test payload, for `spawn`,
//! - slot 2: `Notification` (READ|WRITE) — its children's exit-badge
//!   channel,
//! - slot 3: `Endpoint` (WRITE) — filesystem service (M5.3).
//! - slot 4: `Endpoint` (WRITE) — privileged production stack client
//!   only when BOTH drivers exist (ADR-0040). No app gets this cap.
//!
//! The shell is a plain program: no libc, no allocator — fixed buffers,
//! byte-wise command matching, the shared userspace ABI surface
//! (`userspace/abi.rs`, one wire contract for every ring-3 image),
//! output chunked to the dispatcher's 256-byte debug_write bound. The
//! loop: prompt → `SYS_CONSOLE_READ` (blocks; the kernel echoes what
//! the user types) → match a builtin → answer. Builtins: `help`, `ps`,
//! `echo`, `ls`, `cat NAME`, `write NAME TEXT`, `spawn`, `shutdown`.
//!
//! The file builtins allocate ONE 4 KiB frame lazily on first use
//! (lend-keep pattern, ADR-0022: the owned cap is consumed by the
//! self-map, the LENT copy travels with the FS calls) and reuse it for
//! the rest of the shell's life — a resident process must not leak a
//! frame per command. `write` CREATES a new file and refuses to
//! overwrite an existing one (v1 has no truncate; honest refusal over
//! silent clobber). `cat` streams the file in ≤ 3584-byte chunks.
//!
//! Exit codes (diagnostic contract with the harness): 97 the console
//! refused an output write, 98 `SYS_CONSOLE_READ` returned a typed
//! refusal (impossible for the sole reader — a kernel contract breach),
//! 99 the panic handler ran. The shell otherwise NEVER exits: the
//! machine stops through `shutdown` (Power-gated SYS_SHUTDOWN), not
//! through the shell's death.
#![no_std]
#![no_main]

use core::panic::PanicInfo;
mod permission;

#[path = "../../abi.rs"]
mod abi;
use abi::*;
use arena_lib::net;

// ---- shell-local constants (the grants mirror entry.rs) ----------------------

/// The kernel console's line bound (console::LINE_MAX) — reading with a
/// smaller max would truncate, and the shell has no use for halves.
const LINE_LEN: usize = 120;

const SLOT_POWER: u64 = 0;
const SLOT_IMAGE: u64 = 1; // registry image 0 = the M4.3 test payload
const SLOT_NOTIF: u64 = 2;
const SLOT_FSD: u64 = 3; // M5.3: the filesystem service call side
const SLOT_STACK: u64 = 4; // ADR-0040: privileged admin-only client
const SLOT_MGR_WAKE: u64 = 5; // shared wake hint; NOT stop authority
const SLOT_STACK_DIAG: u64 = 15; // boot-granted R|COPY, not an ordinary client cap
const SLOT_MGR_ADMIN: u64 = 6; // private manager control notification, WRITE only
const SLOT_MEDIATOR: u64 = 16; // ADR-0048: endpoint/W|COPY, full fixture
const SLOT_APPROVAL: u64 = 17; // receiver-verified separate approval marker
const SLOT_PERM_IMAGE: u64 = 18; // trusted test-only delegate fixture, Image25/READ
const SLOT_PACKAGE: u64 = 20; // ADR-0053: receiver request endpoint/W|COPY
const SLOT_PACKAGE_MARKER: u64 = 21; // distinct admin proof R|COPY|DESTROY
// ADR-0044: root-issued diagnostic Process references; never the
// production child's DESTROY. Slot 7 is still the transient child cap.
const SLOT_LIFE_CHILD: u64 = 7;
const SLOT_LIFE_SELF: u64 = 10;
const SLOT_LIFE_MANAGER: u64 = 11;
const SLOT_LIFE_NETD: u64 = 12;
const SLOT_LIFE_RNGD: u64 = 13;
const SLOT_LIFE_FOREIGN: u64 = 14;
const STACK_TEST_TIMER: u64 = 1 << 21;
/// The badge this shell lends its children's exits.
const SPAWN_BADGE: u64 = 0x5AA5;

/// File-I/O window slots: the owned cap (consumed by the self-map) and
/// the LENT copy that travels with the FS calls.
const SLOT_FILE_BUF: u64 = 8;
const SLOT_FILE_LENT: u64 = 9;

const EXIT_CONSOLE_REFUSED: u64 = 98;

// ---- fixed buffers (.bss — the loader's zero-fill is their init) ----------

static mut LINE: [u8; LINE_LEN] = [0; LINE_LEN];
/// (pid, threads) pairs from SYS_PROC_LIST — 32 pairs, the whole table.
static mut PROCS: [u64; 64] = [0; 64];
/// The lazily allocated file window: its VA (0 = not yet allocated).
static mut FILE_VA: u64 = 0;
/// Local copy of service-issued bytes; revocation is at the receiver,
/// never by deleting this array or a kernel cap slot.
static mut PERM_TOKEN: [u8; 16] = [0; 16];
static mut PERM_RETAINED: [u8; 16] = [0; 16];

// ---- byte-wise line helpers (no std, no alloc, no surprises) ---------------

fn eq(line: &[u8], cmd: &[u8]) -> bool {
    if line.len() != cmd.len() {
        return false;
    }
    let mut i = 0;
    while i < line.len() {
        if line[i] != cmd[i] {
            return false;
        }
        i += 1;
    }
    true
}

fn strip_prefix<'a>(line: &'a [u8], p: &[u8]) -> Option<&'a [u8]> {
    if line.len() < p.len() {
        return None;
    }
    let mut i = 0;
    while i < p.len() {
        if line[i] != p[i] {
            return None;
        }
        i += 1;
    }
    Some(&line[p.len()..])
}

/// Split `rest` at the first space: (word, remainder).
fn split_word(rest: &[u8]) -> (&[u8], &[u8]) {
    let mut i = 0;
    while i < rest.len() && rest[i] != b' ' {
        i += 1;
    }
    (&rest[..i], &rest[i..])
}

// ---- the FS plumbing (M5.3) ---------------------------------------------------

/// The file window: allocated/copied/mapped on first use, reused
/// forever after. Returns its VA, or 0 when the kernel refused (the
/// caller reports and moves on — a refused frame is not fatal).
fn file_va(o: &mut Out) -> u64 {
    // SAFETY: FILE_VA is this image's own .bss; the wrappers are the
    // ABI v1 contract; slots mirror the constants above.
    unsafe {
        let va = *core::ptr::addr_of!(FILE_VA);
        if va != 0 {
            return va;
        }
        let phys = syscall1(SYS_ALLOC_FRAME, SLOT_FILE_BUF);
        if phys <= 0 {
            o.str("  frame allocation refused: ");
            o.i64(phys);
            o.crlf();
            return 0;
        }
        if syscall3(SYS_CAP_COPY, SLOT_FILE_BUF, SLOT_FILE_LENT, RIGHTS_ALL) < 0 {
            o.str("  frame cap copy refused\r\n");
            return 0;
        }
        let win = syscall2(SYS_MAP_MEMORY, SLOT_FILE_BUF, 1);
        if win <= 0 {
            o.str("  frame self-map refused\r\n");
            return 0;
        }
        *core::ptr::addr_of_mut!(FILE_VA) = win as u64;
        win as u64
    }
}

/// One synchronous FS request through the granted endpoint (slot 3).
/// `msg` is the IPC v1.1 inline buffer (names IN, dirents OUT).
/// Returns (transport status, reply w0, reply w1).
fn fs_call(op: u64, w1: u64, cap: u64, msg: &mut [u8; MSG_BYTES]) -> (i64, u64, u64) {
    let mut reply = [0u64; 3];
    // SAFETY: wrapper contract; `reply`/`msg` are this thread's own
    // (registered) memory; the endpoint cap is the granted slot 3.
    let r = unsafe {
        syscall6(
            SYS_IPC_CALL,
            SLOT_FSD,
            op,
            w1,
            cap,
            reply.as_mut_ptr() as u64,
            msg.as_mut_ptr() as u64,
        )
    };
    (r, reply[0], reply[1])
}

/// Report a failed FS request honestly (transport vs. service status).
fn fs_error(o: &mut Out, what: &str, r: i64, st: u64) {
    o.str("  ");
    o.str(what);
    if r < 0 {
        o.str(": call refused (");
        o.i64(r);
        o.str(")\r\n");
    } else {
        o.str(": fs status ");
        o.i64(st as i64);
        o.str("\r\n");
    }
}

/// Put a NUL-padded name (or a length word) into the inline message.
fn msg_zero(msg: &mut [u8; MSG_BYTES]) {
    for b in msg.iter_mut() {
        *b = 0;
    }
}

/// ADR-0052: a SECOND actual network client uses the separately linked
/// no_std library (the other is arptest). The existing stack cap is the
/// only authority; this command neither mints a bearer nor gets a new
/// grant. An absent stack is an honest SKIP rather than a fake response.
fn netlib() {
    // An Image/READ is not an Endpoint/WRITE even if it has a valid slot.
    // Refusal is from the kernel's actual kind/rights check, not from a
    // userspace name filter or a guessed process identity.
    if !matches!(
        net::Client::new(SLOT_IMAGE).resolve([10, 0, 2, 2]),
        Err(net::Error::Transport(_))
    ) {
        write_str("m83: netlib FAIL (wrong-kind Image accepted as endpoint)\r\n");
        return;
    }
    write_str("m83: netlib wrong-kind Image endpoint refused by kernel\r\n");
    let mut desc = [0u64; 3];
    if unsafe { syscall2(SYS_CAP_DESCRIBE, SLOT_STACK, desc.as_mut_ptr() as u64) } != 0 {
        write_str("m83: netlib SKIP (no stack endpoint)\r\n");
        return;
    }
    if desc[0] != 2 || desc[2] & RIGHTS_WRITE == 0 {
        write_str("m83: netlib FAIL (stack cap kind/rights)\r\n");
        return;
    }
    match net::Client::new(SLOT_STACK).resolve([10, 0, 2, 2]) {
        Ok(mac) if mac != [0; 6] => {
            write_str("m83: netlib PASS (linked client, live gateway ARP)\r\n")
        }
        _ => write_str("m83: netlib FAIL (real ARP or IPC refused)\r\n"),
    }
}

/// The shell is already the Power-holding administrator. This opt-in
/// integration proof holds its own stack client cap across a real
/// service restart. A TCP/UDP handle is authority by possession, not
/// by pid; the SAME caller's old bearer must be revoked in the fresh
/// server, while the kernel endpoint cap stays valid.
fn stack_call(op: u64, arg: u64) -> (i64, u64, u64) {
    stack_call_cap(op, arg, CAP_NONE)
}

fn stack_call_cap(op: u64, arg: u64, cap: u64) -> (i64, u64, u64) {
    let mut reply = [0u64; 3];
    let status = unsafe {
        syscall6(
            SYS_IPC_CALL,
            SLOT_STACK,
            arg,
            op,
            cap,
            reply.as_mut_ptr() as u64,
            0,
        )
    };
    (status, reply[0], reply[1])
}
fn stack_pause(us: u64) -> bool {
    let timer = unsafe { syscall3(SYS_TIMER_ARM, SLOT_NOTIF, STACK_TEST_TIMER, us) };
    if timer < 0 {
        return false;
    }
    loop {
        let b = unsafe { syscall1(SYS_WAIT, SLOT_NOTIF) };
        if b < 0 {
            return false;
        }
        if b as u64 & STACK_TEST_TIMER != 0 {
            return true;
        }
    }
}
fn stacktest() -> bool {
    let mut cap = [0u64; 3];
    if unsafe { syscall2(SYS_CAP_DESCRIBE, SLOT_STACK, cap.as_mut_ptr() as u64) } != 0
        || cap[0] != 2
        || cap[2] & RIGHTS_WRITE == 0
    {
        write_str("m8: stacktest SKIP (no production client cap)\r\n");
        return false;
    }
    // The endpoint lets us CALL the service, not reap it. `Power` is
    // not a Process cap either; service lifecycle stays manager-owned.
    if unsafe { syscall2(SYS_PROC_FINISH, SLOT_STACK, 1) } >= 0
        || unsafe { syscall2(SYS_PROC_FINISH, SLOT_POWER, 1) } >= 0
    {
        write_str("m8: stacktest FAIL (client cap could stop a process)\r\n");
        return false;
    }
    write_str("m8: stacktest Process-cap stop refused to endpoint-only client\r\n");
    let gateway = 10 | (2 << 16) | (2 << 24); // 10.0.2.2, low byte first
    let (r, st, mac) = stack_call(ARP_OP_RESOLVE, gateway);
    if r < 0 || st != ARP_S_OK || mac == 0 {
        write_str("m8: stacktest FAIL (first real-wire ARP resolve)\r\n");
        return false;
    }
    let (r, st, prior) = stack_call(ARP_OP_STATS, 0);
    if r < 0 || st != ARP_S_OK || prior >> 32 == 0 {
        write_str("m8: stacktest FAIL (first instance has no wire evidence)\r\n");
        return false;
    }
    let (r, st, bearer) = stack_call(UDP_OP_BIND, 5355);
    if r < 0 || st != ARP_S_OK || bearer == 0 {
        write_str("m8: stacktest FAIL (no rngd-backed UDP bearer)\r\n");
        return false;
    }
    write_str(
        "m8: stacktest held old endpoint and issued rngd-backed bearer after real ARP wire work\r\n",
    );
    for cap in [CAP_NONE, SLOT_NOTIF] {
        let (r, st, _) = stack_call_cap(ARP_OP_SHUTDOWN, 0, cap);
        if r != 0 || st != ARP_S_BAD_OP {
            write_str("m8: stacktest FAIL (unauthorized shutdown accepted)\r\n");
            return false;
        }
    }
    write_str("m8: stacktest service refused missing/wrong shutdown marker\r\n");
    let (r, st, _) = stack_call_cap(ARP_OP_SHUTDOWN, 0, SLOT_STACK_DIAG);
    if r < 0 || st != ARP_S_OK {
        write_str("m8: stacktest FAIL (orderly production child exit)\r\n");
        return false;
    }
    write_str("m8: stacktest requested production child exit (manager must reap and restart)\r\n");
    let start = unsafe { syscall0(SYS_CLOCK_NOW) };
    if start < 0 {
        write_str("m8: stacktest FAIL (no monotonic clock)\r\n");
        return false;
    }
    let mut gone = false;
    loop {
        let (r, st, _) = stack_call(UDP_OP_CLOSE, bearer);
        if r == 0 && st == UDP_S_BAD_HANDLE {
            break;
        }
        if r != STATUS_SERVICE_GONE {
            write_str("m8: stacktest FAIL (stale bearer was accepted or unexpected transport)\r\n");
            return false;
        }
        gone = true;
        let now = unsafe { syscall0(SYS_CLOCK_NOW) };
        if now < 0 || now as u64 - start as u64 > 3_000_000 || !stack_pause(25_000) {
            write_str("m8: stacktest FAIL (no bounded replacement)\r\n");
            return false;
        }
    }
    if gone {
        write_str("m8: stacktest observed SERVICE_GONE during child absence\r\n");
    }
    let (r, st, fresh) = stack_call(ARP_OP_STATS, 0);
    if r < 0 || st != ARP_S_OK || fresh >> 32 != 0 {
        write_str("m8: stacktest FAIL (replacement was not fresh)\r\n");
        return false;
    }
    let (r, st, fresh_mac) = stack_call(ARP_OP_RESOLVE, gateway);
    let (r2, st2, after) = stack_call(ARP_OP_STATS, 0);
    if r < 0 || st != ARP_S_OK || mac != fresh_mac || r2 < 0 || st2 != ARP_S_OK || after >> 32 == 0
    {
        write_str(
            "m8: stacktest FAIL (replacement did not perform real post-restart wire work)\r\n",
        );
        return false;
    }
    write_str(
        "m8: stacktest PASS (same endpoint; old bearer revoked; fresh ARP request on real wire)\r\n",
    );
    true
}
/// A Power-gated observation; an Endpoint is not a diagnostic grant.
fn resource_snapshot() -> Option<[u64; 3]> {
    let mut out = [u64::MAX; 3];
    for slot in [SLOT_STACK, 15] {
        // wrong kind and unheld slot
        if unsafe { syscall2(SYS_RESOURCE_SNAPSHOT, slot, out.as_mut_ptr() as u64) } >= 0
            || out != [u64::MAX; 3]
        {
            return None;
        }
    }
    let bad_pointer = unsafe { syscall2(SYS_RESOURCE_SNAPSHOT, SLOT_POWER, 0) };
    if bad_pointer != -3 {
        return None;
    } // STATUS_BAD_ADDRESS
    let ok = unsafe { syscall2(SYS_RESOURCE_SNAPSHOT, SLOT_POWER, out.as_mut_ptr() as u64) };
    if ok != 0 {
        return None;
    }
    Some(out)
}

/// ADR-0042: unlike stacktest's orderly shutdown, this is a genuine
/// user-mode #UD with the caller blocked on an unanswered IPC.
fn stackfault() {
    let mut cap = [0u64; 3];
    if unsafe { syscall2(SYS_CAP_DESCRIBE, SLOT_STACK, cap.as_mut_ptr() as u64) } != 0
        || cap[0] != 2
        || cap[2] & RIGHTS_WRITE == 0
    {
        write_str("m8: stackfault SKIP (no production client cap)\r\n");
        return;
    }
    let mut msg = [0u8; MSG_BYTES];
    let (r, st, _) = fs_call(FS_OP_LS, 0, CAP_NONE, &mut msg);
    if r < 0 || st != FS_OK {
        write_str("m8: stackfault FAIL (filesystem baseline not settled)\r\n");
        return;
    }
    let Some(baseline) = resource_snapshot() else {
        write_str("m8: stackfault FAIL (resource snapshot refused)\r\n");
        return;
    };
    let gateway = 10 | (2 << 16) | (2 << 24);
    let (r, st, mac) = stack_call(ARP_OP_RESOLVE, gateway);
    if r < 0 || st != ARP_S_OK || mac == 0 {
        write_str("m8: stackfault FAIL (no initial real wire)\r\n");
        return;
    }
    let (r, st, bearer) = stack_call(UDP_OP_BIND, 5355);
    if r < 0 || st != ARP_S_OK || bearer == 0 {
        write_str("m8: stackfault FAIL (no original entropy-backed bearer)\r\n");
        return;
    }
    write_str("m8: stackfault issued real-wire request and held bearer before fault\r\n");
    for cap in [CAP_NONE, SLOT_NOTIF] {
        let (r, st, _) = stack_call_cap(ARP_OP_FAULT, 0, cap);
        if r != 0 || st != ARP_S_BAD_OP {
            write_str("m8: stackfault FAIL (unauthorized #UD accepted)\r\n");
            return;
        }
    }
    let (r, st, _) = stack_call(ARP_OP_STATS, 0);
    if r != 0 || st != ARP_S_OK {
        write_str("m8: stackfault FAIL (service lost after refused fault)\r\n");
        return;
    }
    write_str("m8: stackfault service refused missing/wrong diagnostic marker, still live\r\n");
    let (r, _, _) = stack_call_cap(ARP_OP_FAULT, 0, SLOT_STACK_DIAG);
    if r != STATUS_SERVICE_GONE {
        write_str("m8: stackfault FAIL (in-flight call did not receive SERVICE_GONE)\r\n");
        return;
    }
    write_str("m8: stackfault in-flight call answered SERVICE_GONE on actual #UD\r\n");
    let start = unsafe { syscall0(SYS_CLOCK_NOW) };
    if start < 0 {
        write_str("m8: stackfault FAIL (no clock)\r\n");
        return;
    }
    loop {
        let (r, st, _) = stack_call(UDP_OP_CLOSE, bearer);
        if r == 0 && st == UDP_S_BAD_HANDLE {
            break;
        }
        if r != STATUS_SERVICE_GONE {
            write_str("m8: stackfault FAIL (stale bearer accepted or wrong transport)\r\n");
            return;
        }
        let now = unsafe { syscall0(SYS_CLOCK_NOW) };
        if now < 0 || now as u64 - start as u64 > 3_000_000 || !stack_pause(25_000) {
            write_str("m8: stackfault FAIL (no bounded recovery)\r\n");
            return;
        }
    }
    let (r, st, fresh) = stack_call(ARP_OP_STATS, 0);
    if r < 0 || st != ARP_S_OK || fresh >> 32 != 0 {
        write_str("m8: stackfault FAIL (new instance not fresh)\r\n");
        return;
    }
    let (r, st, new_mac) = stack_call(ARP_OP_RESOLVE, gateway);
    let (r2, st2, after) = stack_call(ARP_OP_STATS, 0);
    if r < 0 || st != ARP_S_OK || new_mac != mac || r2 < 0 || st2 != ARP_S_OK || after >> 32 == 0 {
        write_str("m8: stackfault FAIL (no fresh post-fault real-wire work)\r\n");
        return;
    }
    if resource_snapshot() != Some(baseline) {
        write_str("m8: stackfault FAIL (frames/records/processes not flat)\r\n");
        return;
    }
    write_str(
        "m8: stackfault PASS (real #UD, in-flight call failed, same endpoint fresh wire, resources flat)\r\n",
    );
}

/// ADR-0043: ONLY the manager possesses the child's Process handle.
/// This command sends a private request and a separate forgeable wake;
/// the manager must observe a LIVE child and use mode-1 finish itself.
fn depdeny(stall: bool) {
    let mut client = [0u64; 3];
    let mut admin = [0u64; 3];
    if unsafe { syscall2(SYS_CAP_DESCRIBE, SLOT_STACK, client.as_mut_ptr() as u64) } != 0 {
        write_str("m8: dependency probe SKIP (no production client cap)\r\n");
        return;
    }
    if unsafe { syscall2(SYS_CAP_DESCRIBE, SLOT_MGR_ADMIN, admin.as_mut_ptr() as u64) } != 0
        || client[0] != 2
        || admin[0] != 3
        || admin[2] != RIGHTS_WRITE
        || unsafe { syscall2(SYS_PROC_FINISH, SLOT_MGR_ADMIN, 1) } >= 0
    {
        write_str("m8: dependency probe FAIL (private authority mismatch)\r\n");
        return;
    }
    if unsafe {
        syscall2(
            SYS_NOTIFY,
            SLOT_MGR_ADMIN,
            if stall {
                MGR_BADGE_ADMIN_DEPSTALL
            } else {
                MGR_BADGE_ADMIN_DEPFAIL
            },
        )
    } != 0
        || unsafe { syscall2(SYS_NOTIFY, SLOT_MGR_WAKE, MGR_BADGE_ADMIN_WAKE) } != 0
    {
        write_str("m8: dependency probe FAIL (private request refused)\r\n");
        return;
    }
    if stall {
        write_str("m8: depstall private timeout request sent; observing manager\r\n");
    } else {
        write_str("m8: depdeny private failure request sent; observing manager\r\n");
    }
}

fn stackstop() {
    let mut client = [0u64; 3];
    let mut wake = [0u64; 3];
    let mut admin = [0u64; 3];
    if unsafe { syscall2(SYS_CAP_DESCRIBE, SLOT_STACK, client.as_mut_ptr() as u64) } != 0 {
        write_str("m8: stackstop SKIP (no production client cap)\r\n");
        return;
    }
    if unsafe { syscall2(SYS_CAP_DESCRIBE, SLOT_MGR_WAKE, wake.as_mut_ptr() as u64) } != 0
        || unsafe { syscall2(SYS_CAP_DESCRIBE, SLOT_MGR_ADMIN, admin.as_mut_ptr() as u64) } != 0
        || client[0] != 2
        || wake[0] != 3
        || admin[0] != 3
        || client[2] & RIGHTS_WRITE == 0
        || wake[2] != RIGHTS_WRITE
        || admin[2] != RIGHTS_WRITE
        || wake[1] == admin[1]
        || unsafe { syscall2(SYS_PROC_FINISH, SLOT_MGR_ADMIN, 1) } >= 0
        || unsafe { syscall2(SYS_PROC_FINISH, SLOT_MGR_WAKE, 1) } >= 0
        || unsafe { syscall1(SYS_TRY_WAIT, SLOT_MGR_ADMIN) } >= 0
    {
        write_str("m8: stackstop FAIL (private control authority mismatch)\r\n");
        return;
    }
    let mut msg = [0u8; MSG_BYTES];
    let (r, st, _) = fs_call(FS_OP_LS, 0, CAP_NONE, &mut msg);
    if r < 0 || st != FS_OK {
        write_str("m8: stackstop FAIL (filesystem baseline not settled)\r\n");
        return;
    }
    let Some(baseline) = resource_snapshot() else {
        write_str("m8: stackstop FAIL (resource baseline refused)\r\n");
        return;
    };
    let gateway = 10 | (2 << 16) | (2 << 24);
    let (r, st, mac) = stack_call(ARP_OP_RESOLVE, gateway);
    if r < 0 || st != ARP_S_OK || mac == 0 {
        write_str("m8: stackstop FAIL (no initial ARP wire)\r\n");
        return;
    }
    let (r, st, bearer) = stack_call(UDP_OP_BIND, 5355);
    if r < 0 || st != ARP_S_OK || bearer == 0 {
        write_str("m8: stackstop FAIL (no original rngd-backed bearer)\r\n");
        return;
    }
    // A netd WRITE holder could forge this wake bit. Prove the hint
    // alone cannot kill the live child before issuing private STOP.
    if unsafe { syscall2(SYS_NOTIFY, SLOT_MGR_WAKE, MGR_BADGE_ADMIN_WAKE) } != 0
        || !stack_pause(75_000)
    {
        write_str("m8: stackstop FAIL (untrusted wake injection)\r\n");
        return;
    }
    let (r, st, prior) = stack_call(ARP_OP_STATS, 0);
    if r < 0 || st != ARP_S_OK || prior >> 32 == 0 {
        write_str("m8: stackstop FAIL (forgeable wake stopped the child)\r\n");
        return;
    }
    write_str("m8: stackstop forged shared wake alone did NOT stop live child\r\n");
    // A malformed private request is not the exact STOP authorization.
    if unsafe { syscall2(SYS_NOTIFY, SLOT_MGR_ADMIN, MGR_BADGE_ADMIN_STOP << 1) } != 0
        || unsafe { syscall2(SYS_NOTIFY, SLOT_MGR_WAKE, MGR_BADGE_ADMIN_WAKE) } != 0
        || !stack_pause(75_000)
    {
        write_str("m8: stackstop FAIL (malformed control test refused)\r\n");
        return;
    }
    let (r, st, prior) = stack_call(ARP_OP_STATS, 0);
    if r < 0 || st != ARP_S_OK || prior >> 32 == 0 {
        write_str("m8: stackstop FAIL (malformed control stopped the child)\r\n");
        return;
    }
    write_str("m8: stackstop malformed private request did NOT stop live child\r\n");
    if unsafe { syscall2(SYS_NOTIFY, SLOT_MGR_ADMIN, MGR_BADGE_ADMIN_STOP) } != 0
        || unsafe { syscall2(SYS_NOTIFY, SLOT_MGR_WAKE, MGR_BADGE_ADMIN_WAKE) } != 0
    {
        write_str("m8: stackstop FAIL (private administrative request refused)\r\n");
        return;
    }
    write_str("m8: stackstop sent private STOP then shared wake, no Process cap delegated\r\n");
    let start = unsafe { syscall0(SYS_CLOCK_NOW) };
    if start < 0 {
        write_str("m8: stackstop FAIL (no clock)\r\n");
        return;
    }
    loop {
        let (r, st, _) = stack_call(UDP_OP_CLOSE, bearer);
        if r == 0 && st == UDP_S_BAD_HANDLE {
            break;
        }
        if r != STATUS_SERVICE_GONE {
            write_str("m8: stackstop FAIL (old bearer accepted or wrong transport)\r\n");
            return;
        }
        let now = unsafe { syscall0(SYS_CLOCK_NOW) };
        if now < 0 || now as u64 - start as u64 > 3_000_000 || !stack_pause(25_000) {
            write_str("m8: stackstop FAIL (no bounded replacement)\r\n");
            return;
        }
    }
    let (r, st, fresh) = stack_call(ARP_OP_STATS, 0);
    if r < 0 || st != ARP_S_OK || fresh >> 32 != 0 {
        write_str("m8: stackstop FAIL (replacement not fresh)\r\n");
        return;
    }
    let (r, st, new_mac) = stack_call(ARP_OP_RESOLVE, gateway);
    let (r2, st2, after) = stack_call(ARP_OP_STATS, 0);
    if r < 0 || st != ARP_S_OK || new_mac != mac || r2 < 0 || st2 != ARP_S_OK || after >> 32 == 0 {
        write_str("m8: stackstop FAIL (replacement did not perform new real-wire ARP)\r\n");
        return;
    }
    if resource_snapshot() != Some(baseline) {
        write_str("m8: stackstop FAIL (resource totals drifted)\r\n");
        return;
    }
    write_str(
        "m8: stackstop PASS (manager mode-1 stopped live production child, new wire, resources flat)\r\n",
    );
}

/// Both finish modes must refuse a slot that does not convey lifecycle
/// authority over a reappable child. This invokes the ACTUAL kernel
/// syscall, not a host-side imitation of its permission predicate.
fn finish_refused(slot: u64) -> bool {
    let r0 = unsafe { syscall2(SYS_PROC_FINISH, slot, 0) };
    let r1 = unsafe { syscall2(SYS_PROC_FINISH, slot, 1) };
    if r0 != -2 || r1 != -2 {
        let mut o = Out::new();
        o.str("m8: lifetest refusal mismatch slot=");
        o.u64(slot);
        o.str(" mode0=");
        o.i64(r0);
        o.str(" mode1=");
        o.i64(r1);
        o.crlf();
        o.flush();
        return false;
    }
    true
}

/// SYS_PROC_STATUS has six-register ABI slots. Always zero its reserved tail;
/// syscall2 intentionally leaves registers 2–5 unspecified.
fn process_status_query(slot: u64, out: u64) -> i64 {
    unsafe { syscall6(SYS_PROC_STATUS, slot, out, 0, 0, 0, 0) }
}

fn lifetest() {
    let mut client = [0u64; 3];
    if unsafe { syscall2(SYS_CAP_DESCRIBE, SLOT_STACK, client.as_mut_ptr() as u64) } != 0 {
        write_str("m8: lifetest SKIP (no production client cap)\r\n");
        return;
    }
    let mut ids = [0u64; 5];
    for (i, slot) in [
        SLOT_LIFE_SELF,
        SLOT_LIFE_MANAGER,
        SLOT_LIFE_NETD,
        SLOT_LIFE_RNGD,
        SLOT_LIFE_FOREIGN,
    ]
    .iter()
    .enumerate()
    {
        let mut desc = [0u64; 3];
        if unsafe { syscall2(SYS_CAP_DESCRIBE, *slot, desc.as_mut_ptr() as u64) } != 0
            || desc[0] != 4
            || desc[1] == 0
            || desc[2]
                != if i == 4 {
                    RIGHTS_READ
                } else {
                    RIGHTS_READ | RIGHTS_DESTROY
                }
        {
            write_str("m8: lifetest FAIL (literal Process reference or rights mismatch)\r\n");
            return;
        }
        ids[i] = desc[1];
    }
    if ids.iter().enumerate().any(|(i, id)| ids[..i].contains(id)) {
        write_str("m8: lifetest FAIL (duplicate protected or foreign target)\r\n");
        return;
    }
    let mut o = Out::new();
    o.str("m8: lifetest refs shell=");
    o.u64(ids[0]);
    o.str(" manager=");
    o.u64(ids[1]);
    o.str(" netd=");
    o.u64(ids[2]);
    o.str(" rngd=");
    o.u64(ids[3]);
    o.str(" foreign=");
    o.u64(ids[4]);
    o.crlf();
    o.flush();
    let mut msg = [0u8; MSG_BYTES];
    let (r, st, _) = fs_call(FS_OP_LS, 0, CAP_NONE, &mut msg);
    if r < 0 || st != FS_OK {
        write_str("m8: lifetest FAIL (filesystem baseline not settled)\r\n");
        return;
    }
    let gateway = 10 | (2 << 16) | (2 << 24);
    let (r, st, mac) = stack_call(ARP_OP_RESOLVE, gateway);
    let (r2, st2, stats) = stack_call(ARP_OP_STATS, 0);
    if r < 0 || st != ARP_S_OK || mac == 0 || r2 < 0 || st2 != ARP_S_OK || stats >> 32 == 0 {
        write_str("m8: lifetest FAIL (production child not serving actual wire)\r\n");
        return;
    }
    let Some(baseline) = resource_snapshot() else {
        write_str("m8: lifetest FAIL (Power-gated resource baseline)\r\n");
        return;
    };
    for slot in [
        SLOT_LIFE_SELF,
        SLOT_LIFE_MANAGER,
        SLOT_LIFE_NETD,
        SLOT_LIFE_RNGD,
    ] {
        if !finish_refused(slot) {
            write_str("m8: lifetest FAIL (protected target stopped by held DESTROY)\r\n");
            return;
        }
    }
    write_str("m8: lifetest held DESTROY refused for self/manager/netd/rngd\r\n");
    let mut process_status = [u64::MAX; 2];
    if process_status_query(SLOT_LIFE_SELF, process_status.as_mut_ptr() as u64) != 0
        || process_status != [0, 0]
        || process_status_query(SLOT_LIFE_FOREIGN, process_status.as_mut_ptr() as u64) != 0
        || process_status != [0, 0]
        || process_status_query(SLOT_STACK, process_status.as_mut_ptr() as u64) != STATUS_BAD_ARG
        || process_status_query(CAP_SLOTS as u64, process_status.as_mut_ptr() as u64)
            != STATUS_BAD_ARG
        || process_status_query(SLOT_LIFE_SELF, 0) != STATUS_BAD_ADDRESS
    {
        write_str("m8: lifetest FAIL (Process status cap/right/pointer boundary)\r\n");
        return;
    }
    write_str(
        "m8: lifetest Process/READ status live state and wrong-cap/pointer refusals passed\r\n",
    );
    if !finish_refused(SLOT_LIFE_FOREIGN)
        || !finish_refused(SLOT_STACK)
        || !finish_refused(SLOT_POWER)
        || !finish_refused(7)
        || !finish_refused(ids[4])
        || unsafe {
            syscall3(
                SYS_CAP_COPY,
                SLOT_LIFE_FOREIGN,
                SLOT_LIFE_CHILD,
                RIGHTS_READ | RIGHTS_DESTROY,
            )
        } != -2
    {
        write_str("m8: lifetest FAIL (foreign, forged or amplified lifecycle authority)\r\n");
        return;
    }
    write_str("m8: lifetest foreign READ-only/guessed pid/empty/wrong-kind refused\r\n");
    // Our ordinary shell child is the positive control. A Process cap
    // lands in the first free slot (7), never in reserved 10..14.
    let child = unsafe { syscall5(SYS_SPAWN, SLOT_IMAGE, 0, 0, SLOT_NOTIF, SPAWN_BADGE) };
    if child <= 0 {
        write_str("m8: lifetest FAIL (positive-control child spawn refused)\r\n");
        return;
    }
    let badge = unsafe { syscall1(SYS_WAIT, SLOT_NOTIF) };
    let mut desc = [0u64; 3];
    process_status = [u64::MAX; 2];
    if badge != SPAWN_BADGE as i64
        || unsafe { syscall2(SYS_CAP_DESCRIBE, SLOT_LIFE_CHILD, desc.as_mut_ptr() as u64) } != 0
        || desc != [4, child as u64, RIGHTS_READ | RIGHTS_DESTROY]
        || process_status_query(SLOT_LIFE_CHILD, process_status.as_mut_ptr() as u64) != 0
        || process_status != [1, 42]
        || unsafe { syscall2(SYS_PROC_FINISH, SLOT_LIFE_CHILD, 1) } != STATUS_BUSY
        || unsafe { syscall2(SYS_PROC_FINISH, SLOT_LIFE_CHILD, 0) } != 0
        || !finish_refused(SLOT_LIFE_CHILD)
    {
        write_str("m8: lifetest FAIL (live-only mode, positive reap or stale slot)\r\n");
        return;
    }
    if process_status_query(SLOT_LIFE_CHILD, process_status.as_mut_ptr() as u64) != STATUS_BAD_ARG {
        write_str("m8: lifetest FAIL (stale Process status cap remained usable)\r\n");
        return;
    }
    write_str("m8: lifetest status Process/READ reported exact child exit=42; stale refused\r\n");
    write_str("m8: lifetest child reaped by held cap; dead mode-1 and stale both refused\r\n");
    // Resolve a different slirp address after the refusals: the first
    // gateway resolve cannot make this a cache hit. The same endpoint
    // must still drive a new request over the actual virtio-net wire.
    let dns = 10 | (2 << 16) | (3 << 24); // 10.0.2.3
    let (wire_rc, wire_st, dns_mac) = stack_call(ARP_OP_RESOLVE, dns);
    let (r, st, after) = stack_call(ARP_OP_STATS, 0);
    if wire_rc < 0
        || wire_st != ARP_S_OK
        || dns_mac == 0
        || r < 0
        || st != ARP_S_OK
        || after >> 32 <= stats >> 32
        || resource_snapshot() != Some(baseline)
    {
        write_str(
            "m8: lifetest FAIL (foreign production service lost wire or resources drifted)\r\n",
        );
        return;
    }
    write_str(
        "m8: lifetest PASS (protected/foreign/forged/stale denied; own child reaped; wire live; resources flat)\r\n",
    );
}

fn stackstress() {
    let mut cap = [0u64; 3];
    if unsafe { syscall2(SYS_CAP_DESCRIBE, SLOT_STACK, cap.as_mut_ptr() as u64) } != 0
        || cap[0] != 2
        || cap[2] & RIGHTS_WRITE == 0
    {
        write_str("m8: stackstress SKIP (no production client cap)\r\n");
        return;
    }
    // fsd only serves after mount: a synchronous call ensures its
    // asynchronous startup allocations are over before measurement.
    let mut msg = [0u8; MSG_BYTES];
    let (r, st, _) = fs_call(FS_OP_LS, 0, CAP_NONE, &mut msg);
    if r < 0 || st != FS_OK {
        write_str("m8: stackstress FAIL (filesystem baseline not settled)\r\n");
        return;
    }
    let Some(baseline) = resource_snapshot() else {
        write_str("m8: stackstress FAIL (Power-gated snapshot or refusals)\r\n");
        return;
    };
    let mut o = Out::new();
    o.str("m8: stackstress baseline frames=");
    o.u64(baseline[0]);
    o.str(" records=");
    o.u64(baseline[1]);
    o.str(" processes=");
    o.u64(baseline[2]);
    o.crlf();
    o.flush();
    for cycle in 1..=3u64 {
        if !stacktest() {
            write_str("m8: stackstress FAIL (production restart cycle failed)\r\n");
            return;
        }
        let Some(now) = resource_snapshot() else {
            write_str("m8: stackstress FAIL (diagnostic refused after restart)\r\n");
            return;
        };
        let mut o = Out::new();
        o.str("m8: stackstress cycle ");
        o.u64(cycle);
        o.str(" frames=");
        o.u64(now[0]);
        o.str(" records=");
        o.u64(now[1]);
        o.str(" processes=");
        o.u64(now[2]);
        o.crlf();
        o.flush();
        if now != baseline {
            write_str("m8: stackstress FAIL (resource totals not flat)\r\n");
            return;
        }
    }
    write_str("m8: stackstress PASS (3 real-wire restarts, exact frames/records/processes)\r\n");
    let (r, st, _) = stack_call_cap(ARP_OP_SHUTDOWN, 0, SLOT_STACK_DIAG);
    if r < 0 || st != ARP_S_OK {
        write_str("m8: stackstress FAIL (fourth orderly exit refused)\r\n");
        return;
    }
    write_str("m8: stackstress requested fourth exit; budget must leave service OFFLINE\r\n");
}

// ---- the builtins ----------------------------------------------------------

fn do_ps(o: &mut Out) {
    // SAFETY: PROCS is this image's own .bss (32 (pid, threads) pairs);
    // wrapper contract.
    let r = unsafe { syscall2(SYS_PROC_LIST, core::ptr::addr_of!(PROCS) as u64, 32) };
    if r < 0 {
        o.str("  proc_list refused: ");
        o.i64(r);
        o.crlf();
        return;
    }
    // SAFETY: the kernel wrote `r` validated pairs into PROCS.
    unsafe {
        for i in 0..r as usize {
            let pid = *core::ptr::addr_of!(PROCS[2 * i]);
            let threads = *core::ptr::addr_of!(PROCS[2 * i + 1]);
            o.str("  pid ");
            o.u64(pid);
            o.str("  threads ");
            o.u64(threads);
            o.crlf();
            // ADR-0056 adds a resident display service. The bounded Out
            // chunk is 256 bytes, while the fixed process table permits
            // 32 rows. Flush whole rows before saturation rather than
            // silently omitting the shell's later table slot.
            if o.n > 192 {
                o.flush();
            }
        }
    }
}

/// `ls` — walk the FS endpoint's LS cursor to the end, one dirent per
/// inline message.
fn do_ls(o: &mut Out) {
    let mut msg = [0u8; MSG_BYTES];
    let mut cursor: u64 = 0;
    let mut files = 0u64;
    // Bounded walk: the object table holds 32 records; a cursor that
    // never ends is a service bug — report it, never spin.
    let mut steps = 0u32;
    while steps < 33 {
        steps += 1;
        msg_zero(&mut msg);
        let (r, st, _) = fs_call(FS_OP_LS, cursor, CAP_NONE, &mut msg);
        if r < 0 || st != FS_OK {
            fs_error(o, "ls", r, st);
            return;
        }
        // SAFETY: msg is this thread's own buffer; the dirent layout
        // is the FS protocol's (userspace/abi.rs).
        let (next, size, nlen) = unsafe {
            (
                core::ptr::read_unaligned(msg.as_ptr() as *const u32),
                core::ptr::read_unaligned(msg.as_ptr().add(4) as *const u64),
                core::ptr::read_unaligned(msg.as_ptr().add(12) as *const u32),
            )
        };
        if next == FS_CURSOR_END {
            o.str("  ");
            o.u64(files);
            o.str(" file(s)\r\n");
            return;
        }
        o.str("  ");
        o.bytes(&msg[16..16 + nlen as usize]);
        o.str("  ");
        o.u64(size);
        o.str(" bytes\r\n");
        files += 1;
        cursor = u64::from(next);
    }
    o.str("  ls: the cursor never ended (service bug)\r\n");
}

/// `cat NAME` — open, stream the file through the lent frame in
/// ≤ FS_XFER_MAX chunks, close. The bytes travel disk → device → the
/// shell's own frame (fsd forwards the cap; zero copy end to end).
fn do_cat(o: &mut Out, name: &[u8]) {
    if name.is_empty() || name.len() >= FS_NAME_MAX {
        o.str("  usage: cat NAME (1..31 bytes)\r\n");
        return;
    }
    let va = file_va(o);
    if va == 0 {
        return;
    }
    let mut msg = [0u8; MSG_BYTES];
    msg_zero(&mut msg);
    msg[..name.len()].copy_from_slice(name);
    let (r, st, fh) = fs_call(FS_OP_OPEN, 0, CAP_NONE, &mut msg);
    if r < 0 || st != FS_OK {
        fs_error(o, "cat: open", r, st);
        return;
    }
    let mut off: u64 = 0;
    let mut total: u64 = 0;
    loop {
        msg_zero(&mut msg);
        // SAFETY: own buffer; the length word is the protocol's.
        unsafe { core::ptr::write_unaligned(msg.as_mut_ptr() as *mut u64, FS_XFER_MAX) };
        let (r, st, n) = fs_call(FS_OP_READ, fs_rw_w1(fh, off), SLOT_FILE_LENT, &mut msg);
        if r < 0 || st != FS_OK {
            o.flush();
            fs_error(o, "cat: read", r, st);
            break;
        }
        if n == 0 {
            break; // EOF
        }
        // SAFETY: the device DMA'd `n` bytes into the shell's own
        // mapped frame; n <= FS_XFER_MAX < 4096.
        let chunk: &[u8] = unsafe { core::slice::from_raw_parts(va as *const u8, n as usize) };
        // Release the tail held from the previous chunk, then hold this
        // chunk's tail in `o` until a read reports EOF: the file's last
        // bytes and its line end leave in ONE console write (atomic in the
        // kernel), so another process's log line cannot split them.
        o.flush();
        let held = chunk.len() - chunk.len().min(WRITE_MAX - 2);
        write_all(&chunk[..held]);
        o.bytes(&chunk[held..]);
        total += n;
        off += n;
    }
    o.crlf();
    o.flush();
    let (r, st, _) = fs_call(FS_OP_CLOSE, fh, CAP_NONE, &mut msg);
    if r < 0 || st != FS_OK {
        fs_error(o, "cat: close", r, st);
        return;
    }
    o.str("  (");
    o.u64(total);
    o.str(" bytes)\r\n");
}

/// `write NAME TEXT` — CREATE a new file with TEXT as its contents
/// (v1 refuses to overwrite: no truncate yet, and a silent clobber is
/// worse than an honest refusal). The text bytes ride the shell's
/// lent frame; the device DMAs them straight to the disk.
fn do_write(o: &mut Out, rest: &[u8]) {
    let (name, tail) = split_word(rest);
    let text = if tail.first() == Some(&b' ') {
        &tail[1..]
    } else {
        tail
    };
    if name.is_empty() || name.len() >= FS_NAME_MAX {
        o.str("  usage: write NAME TEXT (name 1..31 bytes)\r\n");
        return;
    }
    if text.is_empty() {
        o.str("  write: refusing to create an empty file\r\n");
        return;
    }
    let va = file_va(o);
    if va == 0 {
        return;
    }
    let mut msg = [0u8; MSG_BYTES];
    msg_zero(&mut msg);
    msg[..name.len()].copy_from_slice(name);
    let (r, st, fh) = fs_call(FS_OP_CREATE, 0, CAP_NONE, &mut msg);
    if r < 0 || st != FS_OK {
        if st == FS_ERR_EXISTS {
            o.str("  write: '");
            o.bytes(name);
            o.str("' already exists — v1 has no truncate; refusing to overwrite\r\n");
        } else {
            fs_error(o, "write: create", r, st);
        }
        return;
    }
    // SAFETY: the shell's own mapped frame; text.len() <= LINE_LEN.
    unsafe { core::ptr::copy_nonoverlapping(text.as_ptr(), va as *mut u8, text.len()) };
    msg_zero(&mut msg);
    // SAFETY: own buffer.
    unsafe { core::ptr::write_unaligned(msg.as_mut_ptr() as *mut u64, text.len() as u64) };
    let (r, st, n) = fs_call(FS_OP_WRITE, fs_rw_w1(fh, 0), SLOT_FILE_LENT, &mut msg);
    if r < 0 || st != FS_OK || n != text.len() as u64 {
        fs_error(o, "write", r, st);
        let _ = fs_call(FS_OP_CLOSE, fh, CAP_NONE, &mut msg);
        return;
    }
    let (r, st, _) = fs_call(FS_OP_CLOSE, fh, CAP_NONE, &mut msg);
    if r < 0 || st != FS_OK {
        fs_error(o, "write: close", r, st);
        return;
    }
    o.str("  wrote ");
    o.u64(n);
    o.str(" bytes to '");
    o.bytes(name);
    o.str("'\r\n");
}

/// `put NAME TEXT` — complete CoW replacement, including empty content.
fn do_put(o: &mut Out, rest: &[u8]) {
    let (name, tail) = split_word(rest);
    let text = if tail.first() == Some(&b' ') {
        &tail[1..]
    } else {
        tail
    };
    if name.is_empty() || name.len() >= FS_NAME_MAX {
        o.str("usage: put NAME TEXT\r\n");
        return;
    }
    let va = file_va(o);
    if va == 0 {
        return;
    }
    unsafe {
        core::ptr::write_bytes(va as *mut u8, 0, 4096);
        core::ptr::copy_nonoverlapping(text.as_ptr(), va as *mut u8, text.len());
    }
    let mut msg = [0u8; MSG_BYTES];
    msg[..name.len()].copy_from_slice(name);
    let (r, st, n) = fs_call(
        FS_OP_PUT,
        text.len() as u64,
        if text.is_empty() {
            CAP_NONE
        } else {
            SLOT_FILE_LENT
        },
        &mut msg,
    );
    if r < 0 || st != FS_OK || n != text.len() as u64 {
        fs_error(o, "put", r, st);
        return;
    }
    o.str("  put committed ");
    o.u64(n);
    o.str(" bytes\r\n");
}

/// `rm NAME` — UNLINK (M5.4): one transaction removes the name and
/// queues every sector of the file's extent chain for reclamation two
/// generations later (ADR-0023). An open file is refused honestly —
/// v1 has no unlink-at-last-close.
fn do_rm(o: &mut Out, name: &[u8]) {
    if name.is_empty() || name.len() >= FS_NAME_MAX {
        o.str("  usage: rm NAME (1..31 bytes)\r\n");
        return;
    }
    let mut msg = [0u8; MSG_BYTES];
    msg_zero(&mut msg);
    msg[..name.len()].copy_from_slice(name);
    let (r, st, _) = fs_call(FS_OP_UNLINK, 0, CAP_NONE, &mut msg);
    if r < 0 || st != FS_OK {
        if st == FS_ERR_NOT_FOUND {
            o.str("  rm: no such file: '");
            o.bytes(name);
            o.str("'\r\n");
        } else if st == FS_ERR_BUSY {
            o.str("  rm: '");
            o.bytes(name);
            o.str("' is open — v1 deletes only closed files\r\n");
        } else {
            fs_error(o, "rm", r, st);
        }
        return;
    }
    o.str("  removed '");
    o.bytes(name);
    o.str("'\r\n");
}

fn do_spawn() {
    // Ring-3 contract probe for the caller-cap-only inventory ABI.
    // The boot root decides what occupies these slots; ask the kernel,
    // not the manifest or the process name.
    let mut image = [0u64; 3];
    let mut notif = [0u64; 3];
    let mut endpoint = [0u64; 3];
    // SAFETY: all three output buffers live on this thread's stack.
    let image_ok =
        unsafe { syscall2(SYS_CAP_DESCRIBE, SLOT_IMAGE, image.as_mut_ptr() as u64) } == 0;
    let notif_ok =
        unsafe { syscall2(SYS_CAP_DESCRIBE, SLOT_NOTIF, notif.as_mut_ptr() as u64) } == 0;
    let ep_ok = unsafe { syscall2(SYS_CAP_DESCRIBE, SLOT_FSD, endpoint.as_mut_ptr() as u64) } == 0;
    let power_denied =
        unsafe { syscall2(SYS_CAP_DESCRIBE, SLOT_POWER, image.as_mut_ptr() as u64) } < 0;
    let bad_pointer = unsafe { syscall2(SYS_CAP_DESCRIBE, SLOT_IMAGE, 0) } < 0;
    if !(image_ok
        && image[0] == 1
        && image[1] == 0
        && image[2] & RIGHTS_READ != 0
        && notif_ok
        && notif[0] == 3
        && ep_ok
        && endpoint[0] == 2
        && power_denied
        && bad_pointer)
    {
        let mut o = Out::new();
        o.str("  caller-cap inventory FAILED\r\n");
        o.flush();
        return;
    }
    let mut o = Out::new();
    o.str("  caller-cap inventory grounded; Power and bad pointer refused\r\n");
    o.flush();
    let mut o = Out::new();
    // Empty inheritance spec (null pointer, count 0 — the child needs
    // nothing), and the shell's own notification + badge lent for the
    // child's exit (ADR-0019).
    // SAFETY: wrapper contract; slot numbers mirror entry.rs's grants.
    let r = unsafe { syscall5(SYS_SPAWN, SLOT_IMAGE, 0, 0, SLOT_NOTIF, SPAWN_BADGE) };
    if r <= 0 {
        o.str("  spawn refused: ");
        o.i64(r);
        o.crlf();
        o.flush();
        return;
    }
    o.str("  spawned pid ");
    o.u64(r as u64);
    o.str(" — waiting for its exit badge\r\n");
    o.flush();
    // The child (the untouched M4.3 payload) writes its message to the
    // console while we are parked here — the visible proof of the
    // restart story on real iron.
    // SAFETY: wrapper contract.
    let b = unsafe { syscall1(SYS_WAIT, SLOT_NOTIF) };
    let mut o = Out::new();
    if b == SPAWN_BADGE as i64 {
        o.str("  child exited, badge ");
        o.hex(b as u64);
        o.crlf();
    } else {
        o.str("  wait returned ");
        o.i64(b);
        o.str(" (expected badge ");
        o.hex(SPAWN_BADGE);
        o.str(")\r\n");
    }
    o.flush();
    if b != SPAWN_BADGE as i64 {
        return; // never reap a child still running on a guessed deadline
    }
    // M8.0 substrate: SYS_SPAWN returns a pid, not the Process-cap
    // slot. Discover the actual cap by a caller-cap-only kernel query;
    // possession of Process+DESTROY, not knowing that pid, is authority.
    let mut found = CAP_NONE;
    for slot in 0..16u64 {
        let mut desc = [0u64; 3];
        // SAFETY: our stack owns the 24-byte output for the full call.
        let status = unsafe { syscall2(SYS_CAP_DESCRIBE, slot, desc.as_mut_ptr() as u64) };
        if status == 0 && desc[0] == 4 && desc[1] == r as u64 {
            if found != CAP_NONE {
                let mut o = Out::new();
                o.str("  child cap ambiguous — refusing reap\r\n");
                o.flush();
                return;
            }
            if desc[2] & RIGHTS_DESTROY == 0 {
                let mut o = Out::new();
                o.str("  child cap lacks DESTROY — refusing reap\r\n");
                o.flush();
                return;
            }
            found = slot;
        }
    }
    if found == CAP_NONE {
        let mut o = Out::new();
        o.str("  child Process cap missing — refusing reap\r\n");
        o.flush();
        return;
    }
    // A named Image cap cannot finish a process, even if the pid is
    // known. The correct cap reaps exactly once; its now-vacant slot
    // may be reused by the next spawn without resurrecting authority.
    let forged = unsafe { syscall2(SYS_PROC_FINISH, SLOT_IMAGE, 0) };
    let reaped = unsafe { syscall2(SYS_PROC_FINISH, found, 0) };
    let stale = unsafe { syscall2(SYS_PROC_FINISH, found, 0) };
    let mut o = Out::new();
    if forged < 0 && reaped == 0 && stale < 0 {
        o.str("  child reaped by Process cap; forged/stale refused\r\n");
    } else {
        o.str("  Process-cap lifecycle FAILED: ");
        o.i64(forged);
        o.str("/");
        o.i64(reaped);
        o.str("/");
        o.i64(stale);
        o.crlf();
    }
    o.flush();
}

/// Trusted Power/raw-FS shell is the only package admin; ordinary
/// clients may obtain ONLY the request endpoint. This command merely
/// transfers the distinct marker; `packaged` verifies it at receipt and
/// validates actual AFS1 bytes before acknowledging any staged record.
fn package_command(o: &mut Out, rest: &[u8]) {
    if eq(rest, b"restart")
        || eq(rest, b"installtest")
        || eq(rest, b"selecttest")
        || eq(rest, b"selectlite")
        || eq(rest, b"upgradetest")
        || eq(rest, b"deathtest")
        || eq(rest, b"deathfault")
        || eq(rest, b"oldlive")
        || eq(rest, b"resources")
        || eq(rest, b"installtwo")
        || eq(rest, b"fourthtest")
        || eq(rest, b"maximalselect")
        || eq(rest, b"graphics")
        || eq(rest, b"graphicsrevoke")
    {
        let installtest = eq(rest, b"installtest");
        let selecttest = eq(rest, b"selecttest");
        let selectlite = eq(rest, b"selectlite");
        let upgradetest = eq(rest, b"upgradetest");
        let deathtest = eq(rest, b"deathtest");
        let deathfault = eq(rest, b"deathfault");
        let oldlive = eq(rest, b"oldlive");
        let resources = eq(rest, b"resources");
        let installtwo = eq(rest, b"installtwo");
        let fourthtest = eq(rest, b"fourthtest");
        let maximalselect = eq(rest, b"maximalselect");
        let graphics = eq(rest, b"graphics");
        let graphicsrevoke = eq(rest, b"graphicsrevoke");
        if resources {
            if let Some([frames, records, processes]) = resource_snapshot() {
                o.str("pkg: observed frames=");
                o.u64(frames);
                o.str(" records=");
                o.u64(records);
                o.str(" processes=");
                o.u64(processes);
                o.crlf();
            } else {
                o.str("pkg: resource snapshot refused\r\n");
            }
            return;
        }
        let mut private = [0u64; 3];
        let mut wake = [0u64; 3];
        let mut endpoint = [0u64; 3];
        if unsafe {
            syscall2(
                SYS_CAP_DESCRIBE,
                SLOT_MGR_ADMIN,
                private.as_mut_ptr() as u64,
            )
        } != 0
            || unsafe { syscall2(SYS_CAP_DESCRIBE, SLOT_MGR_WAKE, wake.as_mut_ptr() as u64) } != 0
            || unsafe { syscall2(SYS_CAP_DESCRIBE, SLOT_PACKAGE, endpoint.as_mut_ptr() as u64) }
                != 0
            || private[0] != 3
            || private[2] != RIGHTS_WRITE
            || wake[0] != 3
            || wake[2] != RIGHTS_WRITE
            || private[1] == wake[1]
            || endpoint[0] != 2
            || endpoint[2] != RIGHTS_WRITE | RIGHTS_COPY
        {
            o.str("pkg: private restart authority unavailable\r\n");
            return;
        }
        let badge = if graphics {
            MGR_BADGE_ADMIN_PKG_GRAPHICS
        } else if graphicsrevoke {
            MGR_BADGE_ADMIN_PKG_GRAPHICS_REVOKE
        } else if installtest {
            MGR_BADGE_ADMIN_PKG_INSTALLTEST
        } else if selecttest {
            MGR_BADGE_ADMIN_PKG_SELECTTEST
        } else if selectlite {
            MGR_BADGE_ADMIN_PKG_SELECTLITE
        } else if upgradetest {
            MGR_BADGE_ADMIN_PKG_UPGRADETEST
        } else if maximalselect {
            MGR_BADGE_ADMIN_PKG_MAXIMAL_SELECT
        } else if installtwo {
            MGR_BADGE_ADMIN_PKG_INSTALL_TWO
        } else if fourthtest {
            MGR_BADGE_ADMIN_PKG_FOURTH
        } else if oldlive {
            MGR_BADGE_ADMIN_PKG_OLDLIVE
        } else if deathfault {
            MGR_BADGE_ADMIN_PKG_DEATHFAULT
        } else if deathtest {
            MGR_BADGE_ADMIN_PKG_DEATHTEST
        } else {
            MGR_BADGE_ADMIN_PKG_RESTART
        };
        if unsafe { syscall2(SYS_NOTIFY, SLOT_MGR_ADMIN, badge) } != 0
            || unsafe { syscall2(SYS_NOTIFY, SLOT_MGR_WAKE, MGR_BADGE_ADMIN_WAKE) } != 0
        {
            o.str(
                if installtest
                    || selecttest
                    || selectlite
                    || upgradetest
                    || deathtest
                    || deathfault
                    || oldlive
                    || installtwo
                    || fourthtest
                    || maximalselect
                {
                    "pkg: private manager lifecycle fixture request refused\r\n"
                } else {
                    "pkg: private restart request refused\r\n"
                },
            );
            return;
        }
        o.str(if installtest {
            "pkg: fixed signed INSTALL fixture requested; no receipt yet\r\n"
        } else if selecttest || selectlite {
            "pkg: fixed signed SELECT fixture requested; no receipt yet\r\n"
        } else if upgradetest {
            "pkg: fixed signed version-eight UPGRADE fixture requested; no receipt yet\r\n"
        } else if installtwo || fourthtest || maximalselect {
            "pkg: maximal historical platter fixture requested; no receipt yet\r\n"
        } else if oldlive {
            "pkg: signed old-child live cutover fixture requested; no receipt yet\r\n"
        } else if deathtest || deathfault {
            "pkg: fatal manager-death negative fixture requested; no PASS expected\r\n"
        } else {
            "pkg: private manager restart requested; no receipt yet\r\n"
        });
        return;
    }
    let (verb, remainder) = split_word(rest);
    // split_word deliberately retains the separator (unlike strip_prefix).
    if let Some(id) = remainder.strip_prefix(b" ") {
        package_call(o, verb, id);
    } else {
        o.str("pkg: use query|stage|policy ID\r\n");
    }
}
fn package_call(o: &mut Out, verb: &[u8], id: &[u8]) {
    // Diagnostic variants deliberately exercise the SAME stage opcode
    // without approval, with a different marker, or with attenuated
    // marker rights. They cannot mutate the AFS1 namespace.
    let (op, admin, diag_cap) = if eq(verb, b"query") {
        (PKG_OP_QUERY, false, CAP_NONE)
    } else if eq(verb, b"stage") {
        (PKG_OP_STAGE, true, SLOT_PACKAGE_MARKER)
    } else if eq(verb, b"policy") {
        (PKG_OP_POLICY, true, SLOT_PACKAGE_MARKER)
    } else if eq(verb, b"stage-noauth") {
        (PKG_OP_STAGE, false, CAP_NONE)
    } else if eq(verb, b"stage-wrong") {
        (PKG_OP_STAGE, false, SLOT_APPROVAL)
    } else if eq(verb, b"stage-wrongkind") {
        (PKG_OP_STAGE, false, SLOT_MEDIATOR)
    } else if eq(verb, b"stage-attenuated") {
        (PKG_OP_STAGE, true, 22)
    } else if eq(verb, b"install-wrong") {
        (PKG_OP_INSTALL, false, SLOT_PACKAGE_MARKER)
    } else if eq(verb, b"install-noauth") {
        (PKG_OP_INSTALL, false, CAP_NONE)
    } else {
        o.str("pkg: use query|stage|policy ID\r\n");
        return;
    };
    if id.is_empty() || id.len() > 31 || id.contains(&b' ') {
        o.str("pkg: ID must be 1..31 ASCII bytes, no spaces\r\n");
        return;
    }
    let mut ep = [0u64; 3];
    let mut marker = [0u64; 3];
    let mut other = [0u64; 3];
    if unsafe { syscall2(SYS_CAP_DESCRIBE, SLOT_PACKAGE, ep.as_mut_ptr() as u64) } != 0
        || ep[0] != 2
        || ep[2] != RIGHTS_WRITE | RIGHTS_COPY
        || unsafe { syscall2(SYS_CAP_DESCRIBE, SLOT_MEDIATOR, other.as_mut_ptr() as u64) } != 0
        || ep[1] == other[1]
        || admin
            && (unsafe {
                syscall2(
                    SYS_CAP_DESCRIBE,
                    SLOT_PACKAGE_MARKER,
                    marker.as_mut_ptr() as u64,
                )
            } != 0
                || marker[0] != 3
                || marker[2] != RIGHTS_READ | RIGHTS_COPY | RIGHTS_DESTROY
                || unsafe { syscall2(SYS_CAP_DESCRIBE, SLOT_APPROVAL, other.as_mut_ptr() as u64) }
                    != 0
                || marker[1] == other[1])
    {
        o.str("pkg: package authority unavailable (OFFLINE)\r\n");
        return;
    }
    if diag_cap == 22 {
        let mut attenuated = [0u64; 3];
        let held = unsafe { syscall2(SYS_CAP_DESCRIBE, 22, attenuated.as_mut_ptr() as u64) };
        if held != 0 {
            // Test-only local R|C cap occupies ONE fixed shell slot until
            // shell exit: it has no DESTROY and cannot be removed locally.
            // Its IPC-landed copy must be destroyed by the receiver.
            if unsafe {
                syscall3(
                    SYS_CAP_COPY,
                    SLOT_PACKAGE_MARKER,
                    22,
                    RIGHTS_READ | RIGHTS_COPY,
                )
            } != 0
                || unsafe { syscall2(SYS_CAP_DESCRIBE, 22, attenuated.as_mut_ptr() as u64) } != 0
            {
                o.str("pkg: attenuated diagnostic cap unavailable\r\n");
                return;
            }
        }
        if attenuated != [3, marker[1], RIGHTS_READ | RIGHTS_COPY] {
            o.str("pkg: attenuated diagnostic source mismatch\r\n");
            return;
        }
    }
    let mut msg = [0u8; MSG_BYTES];
    msg[..id.len()].copy_from_slice(id);
    let mut reply = [0u64; 3];
    let rc = unsafe {
        syscall6(
            SYS_IPC_CALL,
            SLOT_PACKAGE,
            op,
            if op == PKG_OP_INSTALL { 1 } else { 0 },
            diag_cap,
            reply.as_mut_ptr() as u64,
            msg.as_mut_ptr() as u64,
        )
    };
    if rc < 0 {
        o.str("pkg: transport refused ");
        o.i64(rc);
        o.crlf();
        return;
    }
    if reply[2] != CAP_NONE {
        let dropped = unsafe { syscall1(SYS_CAP_DESTROY, reply[2]) };
        o.str(if dropped == 0 {
            "pkg: unexpected reply cap destroyed\r\n"
        } else {
            "pkg: unexpected reply cap disposal FAILED\r\n"
        });
        return;
    }
    o.str("pkg: ");
    if reply[0] == PKG_ELIGIBLE {
        o.str("ELIGIBLE (staged, NOT installed/active) version ");
        o.u64(reply[1]);
        o.str(" digest ");
        for b in &msg[..32] {
            o.hex2(*b);
        }
    } else if reply[0] == PKG_UNSET {
        o.str("UNSET");
    } else if reply[0] == PKG_INELIGIBLE {
        o.str("INELIGIBLE");
    } else if reply[0] == PKG_OK && op == PKG_OP_POLICY {
        o.str("POLICY committed generation ");
        o.u64(reply[1]);
    } else {
        o.str("refused status ");
        o.i64(reply[0] as i64);
    }
    o.crlf();
}

// ---- texts -----------------------------------------------------------------

const BANNER: &str = "ArenaOS shell v0.10 (M4.6 + M5.3/5.4 + M6.5, ADR-0020/0023/0028) — serial, keyboard, or console port.\r\n";
const PROMPT: &str = "arena> ";
/// Two bounded debug-write chunks: Out::push drops bytes beyond
/// WRITE_MAX, so never append to the old near-full help buffer.
const HELP: &str = "commands:\r\n  help - this text\r\n  ps - live processes\r\n  echo TEXT - print TEXT\r\n  ls - list the AFS1 files\r\n  cat NAME - print a file\r\n  write NAME TXT - create a file\r\n  rm NAME - delete a file\r\n  spawn - run image 0\r\n";
const HELP_MORE: &str = "  stacktest - privileged stack restart proof\r\n  stackstress - destructive restart budget and accounting test\r\n  stackfault - opt-in in-flight #UD crash recovery\r\n  netlib - linked native client, live gateway ARP\r\n";
const HELP_LAST: &str = "  stackstop - opt-in manager-owned forced live stop\r\n  lifetest - opt-in Process-cap refusal audit\r\n  depdeny - failed driver probe\r\n  depstall - blocked driver probe\r\n  perm request|allow|deny|revoke|acquire|read - durable mediated access\r\n  pkg query|policy|stage ID - signed STAGING ONLY, no install\r\n  pkg restart - Power-shell manager Process-cap lifecycle proof\r\n  shutdown - halt the machine\r\n";

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    // This image has no panicking path by construction (checked
    // indexing-free arithmetic, no allocation). If a future shell
    // reaches here, say so through the ABI itself.
    write_str("ARENAOS-SHELL: PANIC\r\n");
    // SAFETY: thread_exit diverges.
    unsafe { syscall1(SYS_THREAD_EXIT, EXIT_PANIC) };
    loop {
        core::hint::spin_loop();
    }
}

// ---- the program -----------------------------------------------------------

/// The shell's event loop. Entered by the spawn protocol's first thread
/// at ring 3 with RSP = the derived stack top; nothing here returns —
/// the machine stops through `shutdown`, and the only other exits are
/// the diagnostic codes above.
///
/// # Safety
/// As every image's `_start`: ring 3, derived stack top, kernel-loaded
/// address space, the documented grants (slots 0..3). The body is
/// ABI v1 wrappers over this image's own statics, single-threaded,
/// every static touched through addr_of/addr_of_mut only.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _start() -> ! {
    // SAFETY: this is the whole program — ABI v1 wrappers over the
    // shell's own statics, inside the address space the kernel loaded,
    // validated, and handed to ring 3. Single-threaded, no aliases:
    // every static is touched through addr_of/addr_of_mut only.
    unsafe {
        write_str(BANNER);
        loop {
            write_str(PROMPT);
            let n = syscall2(
                SYS_CONSOLE_READ,
                core::ptr::addr_of!(LINE) as u64,
                LINE_LEN as u64,
            );
            if n < 0 {
                // Typed refusal for the sole reader is a kernel contract
                // breach — report it and die diagnostically, never spin.
                let mut o = Out::new();
                o.str("console_read refused: ");
                o.i64(n);
                o.crlf();
                o.flush();
                syscall2(SYS_THREAD_EXIT, EXIT_CONSOLE_REFUSED, 0);
            }
            if n == 0 {
                continue; // blank line: straight back to the prompt
            }
            let line =
                core::slice::from_raw_parts(core::ptr::addr_of!(LINE).cast::<u8>(), n as usize);

            let mut o = Out::new();
            if eq(line, b"help") {
                o.str(HELP);
                o.flush();
                write_str(HELP_MORE);
                write_str(HELP_LAST);
                continue;
            } else if eq(line, b"ps") {
                do_ps(&mut o);
            } else if eq(line, b"echo") {
                o.crlf();
            } else if let Some(rest) = strip_prefix(line, b"echo ") {
                o.bytes(rest);
                o.crlf();
            } else if eq(line, b"ls") {
                do_ls(&mut o);
            } else if let Some(rest) = strip_prefix(line, b"cat ") {
                do_cat(&mut o, rest);
            } else if let Some(rest) = strip_prefix(line, b"put ") {
                do_put(&mut o, rest);
            } else if let Some(rest) = strip_prefix(line, b"write ") {
                do_write(&mut o, rest);
            } else if let Some(rest) = strip_prefix(line, b"rm ") {
                do_rm(&mut o, rest);
            } else if let Some(rest) = strip_prefix(line, b"perm ") {
                permission::dispatch(&mut o, rest);
            } else if let Some(rest) = strip_prefix(line, b"pkg ") {
                package_command(&mut o, rest);
            } else if eq(line, b"netlib") {
                o.flush();
                netlib();
                continue;
            } else if eq(line, b"stacktest") {
                o.flush();
                let _ = stacktest();
                continue;
            } else if eq(line, b"stackstress") {
                o.flush();
                stackstress();
                continue;
            } else if eq(line, b"stackfault") {
                o.flush();
                stackfault();
                continue;
            } else if eq(line, b"stackstop") {
                o.flush();
                stackstop();
                continue;
            } else if eq(line, b"depdeny") {
                o.flush();
                depdeny(false);
                continue;
            } else if eq(line, b"depstall") {
                o.flush();
                depdeny(true);
                continue;
            } else if eq(line, b"lifetest") {
                o.flush();
                lifetest();
                continue;
            } else if eq(line, b"spawn") {
                o.flush();
                do_spawn(); // does its own output (the child talks mid-flight)
                continue;
            } else if eq(line, b"shutdown") {
                o.str("shutting down...\r\n");
                o.flush();
                let r = syscall1(SYS_SHUTDOWN, SLOT_POWER);
                // Only reachable if the kernel refused the Power cap —
                // report honestly and keep serving.
                let mut o = Out::new();
                o.str("shutdown refused: ");
                o.i64(r);
                o.crlf();
            } else {
                o.str("unknown command: '");
                o.bytes(line);
                o.str("' — try 'help'\r\n");
            }
            o.flush();
        }
    }
}
