//! Authority/lifecycle service. Presentation belongs in shell.rs and compose.rs.
#![no_std]
#![no_main]
#![allow(clippy::deref_addrof, clippy::collapsible_if)]
use arena_desktop::{
    compose::{self, Damage, PopupScene, Scene, WindowScene},
    model as wm,
    shell::Shell,
};
use arena_desktop::{
    input_wire,
    model::{Action, State},
    wire::Frame,
};
use arena_gfxkit::Canvas;
use arena_process::{
    handles::{Handle, InsertError},
    process::{
        ChildProcess, ExitSignal, FinishMode, GroupSpawnError, InheritGrant, ProcessGroup,
        SpawnError,
    },
};
use arena_startup_abi::{
    manifest::{FLAG_MULTI_INSTANCE, FLAG_STANDARD_STREAMS},
    startup::{CapabilityDescriptor, CapabilityRole},
};
use core::panic::PanicInfo;
#[path = "../../../abi.rs"]
mod abi;
use abi::*;
const DISPLAY: u64 = 0;
const SERVER: u64 = 1;
const INPUT: u64 = 2;
const POOL: u64 = 3;
const CLOCK: u64 = 4;
const CALL_SIDE: u64 = 5;
const APPLICATION: u64 = 6;
/// ABI-v2 child slot layout: startup transport 0, per-session badged endpoint
/// 1, writable surface 2, private clock 3, optional explicit tail grant 4.
const V2_ENDPOINT_SLOT: u64 = 1;
const V2_SURFACE_SLOT: u64 = 2;
const V2_CLOCK_SLOT: u64 = 3;
const V2_BADGE_RIGHTS: u64 = RIGHTS_WRITE | RIGHTS_COPY | RIGHTS_DESTROY;
/// filesd's /Users/user record (ADR-0077), minted by the kernel.
const USER_ROOT: u64 = 20;
/// Direct, write-only endpoint to the receiver-verified package service (ADR-0091).
const PACKAGE_ENDPOINT: u64 = 43;
/// Narrow boot grant for creating helper-private notifications.
const NOTIFICATION_FACTORY_SLOT: u64 = 44;
const PIXEL_OFFSET: usize = 4096;
const LIMIT: usize = wm::MAX_WINDOWS;
/// A live application instance owns its own exact-capability process group.
/// Four members matches the reusable ArenaOS lifecycle model and leaves room
/// for the primary plus explicitly authorized helpers.
const APP_GROUP_PROCESSES: usize = 4;
const STREAM_INPUT_PENDING: usize = 64;
/// Window presentation records are separate from process sessions. The WM
/// remains the total-window bound; this table can hold all ordinary windows
/// beyond one primary window per process.
const EXTRA_LIMIT: usize = wm::MAX_WINDOWS;
const _: () = assert!(LIMIT == arena_desktop::apps::STARTUP_INSTANCE_SLOTS);
/// Session clocks and exact-cap scans follow the shared kernel ABI width.
/// Pages of one bounded transient surface (ADR-0075); the final pages of a
/// session's shared reservation and of its private snapshot.
const TRANSIENT_PAGES: u64 = (wm::TRANSIENT_MAX_PIXELS * 4 / 4096) as u64;
/// The session's filesd I/O page, last in the reservation (ADR-0077).
const FILE_PAGES: u64 = arena_desktop::client::FILE_PAGES as u64;
/// Per-session memory, fixed for the screen at startup (ADR-0075): every
/// session can take any size up to the work area (maximize) without
/// reallocation, so a resize never changes which memory backs a session.
#[derive(Clone, Copy)]
struct Reserve {
    /// Main surface pages (work area, rounded up).
    surface_pages: u64,
    /// Shared region: I/O page + main surface + transient surface + the
    /// client's filesd page.
    shared_pages: u64,
    /// Private snapshot region: main surface + transient surface.
    snapshot_pages: u64,
}
static mut RESERVE: Reserve = Reserve {
    surface_pages: 0,
    shared_pages: 0,
    snapshot_pages: 0,
};
/// Private client clock of session `i`: slots 7..=12 and 14..=19, then
/// 21..=29 and 32..=42. Slot 13 is the filesystem endpoint, slot 20 the
/// filesd lineage capability, and 30..=31 stay out of the clock range for
/// the first-free display IPC reply. Filesd scratch uses 125..=126.
fn clock(i: usize) -> u64 {
    if i < 6 {
        7 + i as u64
    } else if i < 12 {
        8 + i as u64
    } else if i < 21 {
        9 + i as u64
    } else {
        11 + i as u64
    }
}
fn audit_session_clocks() {
    let mut objects = [u64::MAX; LIMIT];
    for i in 0..LIMIT {
        let slot = clock(i);
        let Some(desc) = describe(slot) else {
            log(b"[desktop] clock audit missing cap at slot=");
            log_number(slot);
            log(b"\n");
            die(96)
        };
        if slot >= CAP_SLOTS as u64
            || matches!(slot, 13 | USER_ROOT | 30 | 31)
            || desc[0] != 3
            || desc[2] != (RIGHTS_READ | RIGHTS_WRITE | RIGHTS_COPY)
            || objects[..i].contains(&desc[1])
        {
            log(b"[desktop] clock audit mismatch index/slot/kind/object/rights=");
            log_number(i as u64);
            log(b"/");
            log_number(slot);
            for word in desc {
                log(b"/");
                log_number(word);
            }
            log(b"\n");
            die(96)
        }
        objects[i] = desc[1];
    }
    let Some(frame_clock) = describe(CLOCK) else {
        log(b"[desktop] clock audit missing frame clock\n");
        die(96)
    };
    let Some(fs_endpoint) = describe(13) else {
        log(b"[desktop] clock audit missing filesd endpoint slot13\n");
        die(96)
    };
    let Some(user_root) = describe(USER_ROOT) else {
        log(b"[desktop] clock audit missing filesd lineage slot20\n");
        die(96)
    };
    let package_endpoint = cap_slot_descriptor(PACKAGE_ENDPOINT as usize);
    let notification_factory = cap_slot_descriptor(NOTIFICATION_FACTORY_SLOT as usize);
    let frame_slot = unsafe {
        syscall6(
            SYS_CAP_OCCUPIED,
            arena_desktop::fs_backend::FILE_FRAME_SLOT,
            0,
            0,
            0,
            0,
            0,
        )
    };
    let lent_slot = unsafe {
        syscall6(
            SYS_CAP_OCCUPIED,
            arena_desktop::fs_backend::FILE_FRAME_LENT_SLOT,
            0,
            0,
            0,
            0,
            0,
        )
    };
    if frame_clock[0] != 3
        || objects.contains(&frame_clock[1])
        || fs_endpoint[0] != 2
        || fs_endpoint[2] != RIGHTS_WRITE
        || user_root[0] != 12
        || user_root[2] != (RIGHTS_WRITE | RIGHTS_COPY)
        || (package_endpoint[0] != 0
            && (package_endpoint[0] != 2
                || package_endpoint[2] != RIGHTS_WRITE
                || package_endpoint[1] == fs_endpoint[1]
                || package_endpoint[1] == user_root[1]))
        || notification_factory
            != [
                u64::from(CAP_KIND_NOTIFICATION_FACTORY),
                0,
                u64::from(RIGHTS_WRITE),
            ]
        || frame_slot != 0
        || lent_slot != 0
    {
        log(b"[desktop] fixed-cap audit frame=");
        for word in frame_clock {
            log_number(word);
            log(b"/");
        }
        for word in fs_endpoint {
            log_number(word);
            log(b"/");
        }
        for word in user_root {
            log_number(word);
            log(b"/");
        }
        log_number(frame_slot as u64);
        log(b"/");
        log_number(lent_slot as u64);
        log(b"\n");
        die(96)
    }
    log(b"[desktop] audited 32 distinct client clocks; filesystem endpoint slot13; filesd lineage slot20; package endpoint slot43 (optional); private notification factory slot44; scratch slots125/126 free\n");
}
/// Presentation state of a session's transient surface, valid only while
/// the window policy still holds a popup with this handle.
#[derive(Clone, Copy)]
struct PopupState {
    handle: u64,
    published: bool,
    content: u64,
    regions: compose::Regions,
}
#[derive(Clone, Copy)]
struct HelperMember {
    handle: u32,
    helper_id: [u8; 32],
    timer_slot: u8,
    active: bool,
}
const EMPTY_HELPER: HelperMember = HelperMember {
    handle: 0,
    helper_id: [0; 32],
    timer_slot: u8::MAX,
    active: false,
};
const HELPERS_PER_INSTANCE: usize = APP_GROUP_PROCESSES - 1;
const NO_POPUP: PopupState = PopupState {
    handle: 0,
    published: false,
    content: 0,
    regions: compose::Regions::NONE,
};
#[derive(Clone, Copy)]
struct Session {
    id: u64,
    va: u64,
    stream_cap: u64,
    stream_va: u64,
    stream_faulted: bool,
    stream_eof: u8,
    stdin_pending: [u8; STREAM_INPUT_PENDING],
    stdin_head: u8,
    stdin_count: u8,
    process: Option<Handle>,
    handle: u64,
    title: [u8; 32],
    published: bool,
    kind: u8,
    /// Descriptive identity for registry-backed active-instance indicators;
    /// the held Process capability remains the only process authority.
    application_id: [u8; 32],
    /// Exact package startup flags retained for explicitly launched helpers.
    app_flags: u32,
    /// Helpers are process-group members but have no window session.
    helpers: [HelperMember; HELPERS_PER_INSTANCE],
    scope: u8,
    /// Rights formerly carried by the attenuated function SharedRegion cap;
    /// now trusted session policy used only after kernel badge validation.
    function_rights: u64,
    /// Monotonic, nonzero badge of this session's inherited service endpoint.
    badge: u32,
    launch_targets: u8,
    path: [u8; 32],
    close_pending: bool,
    ending: bool,
    reveal: arena_ui::motion::Motion,
    focus: arena_ui::motion::Motion,
    reveal_last: i32,
    focus_last: i32,
    /// Advances on every authenticated Damage (published raster changed).
    content: u64,
    /// Published regions since the last presented frame (Phase 11.1).
    regions: compose::Regions,
    /// Private snapshot mapping (held by its mapping pin only, no cap).
    snapshot: u64,
    /// Size of the published main raster (Phase 11.3).
    surface: (u16, u16),
    popup: PopupState,
    /// Head of this session's filesd lineage (held by the broker only, no
    /// rights; revoking it retires every file capability of the session).
    files_head: u64,
    /// The chooser's outcome is ready for TakeGrant (granted or cancelled).
    grant_ready: bool,
    /// The capability the chooser granted (CAP_NONE: cancelled), its title
    /// (display only) and whether it was a save.
    grant: u64,
    grant_title: [u8; 32],
    grant_save: bool,
    grant_read_only: bool,
    /// A File capability offered by this exact authenticated session.
    offered_file: u64,
}
#[derive(Clone, Copy)]
struct ExtraWindow {
    owner_session: usize,
    handle: u64,
    /// Full descriptive SharedRegion generation, used only as a unique
    /// surface key in the compositor model.
    backing_id: u64,
    va: u64,
    snapshot: u64,
    title: [u8; 32],
    published: bool,
    surface: (u16, u16),
    content: u64,
    regions: compose::Regions,
    popup: PopupState,
    close_pending: bool,
    ending: bool,
    reveal: arena_ui::motion::Motion,
    focus: arena_ui::motion::Motion,
    reveal_last: i32,
    focus_last: i32,
}
const EMPTY_EXTRA: ExtraWindow = ExtraWindow {
    owner_session: usize::MAX,
    handle: 0,
    backing_id: 0,
    va: 0,
    snapshot: 0,
    title: [0; 32],
    published: false,
    surface: (0, 0),
    content: 0,
    regions: compose::Regions::NONE,
    popup: NO_POPUP,
    close_pending: false,
    ending: false,
    reveal: arena_ui::motion::Motion::fixed(arena_ui::metrics::TITLE_HEIGHT),
    focus: arena_ui::motion::Motion::fixed(0),
    reveal_last: arena_ui::metrics::TITLE_HEIGHT,
    focus_last: 0,
};
const EMPTY: Session = Session {
    id: 0,
    va: 0,
    stream_cap: CAP_NONE,
    stream_va: 0,
    stream_faulted: false,
    stream_eof: 0,
    stdin_pending: [0; STREAM_INPUT_PENDING],
    stdin_head: 0,
    stdin_count: 0,
    process: None,
    handle: 0,
    title: [0; 32],
    published: false,
    kind: 5,
    application_id: [0; 32],
    app_flags: 0,
    helpers: [EMPTY_HELPER; HELPERS_PER_INSTANCE],
    scope: 0,
    function_rights: 0,
    badge: 0,
    launch_targets: 0,
    path: [0; 32],
    close_pending: false,
    ending: false,
    reveal: arena_ui::motion::Motion::fixed(arena_ui::metrics::TITLE_HEIGHT),
    focus: arena_ui::motion::Motion::fixed(0),
    reveal_last: arena_ui::metrics::TITLE_HEIGHT,
    focus_last: 0,
    content: 0,
    regions: compose::Regions::NONE,
    snapshot: 0,
    surface: (0, 0),
    popup: NO_POPUP,
    files_head: CAP_NONE,
    grant_ready: false,
    grant: CAP_NONE,
    grant_title: [0; 32],
    grant_save: false,
    grant_read_only: false,
    offered_file: CAP_NONE,
};

/// Temporary owner of a new stream page during launch. A failed launch
/// removes its mapping and exact owner cap; success transfers both resources
/// into the AppInstance record.
struct PendingStream {
    cap: u64,
    va: u64,
}

impl PendingStream {
    fn create() -> Result<Self, i64> {
        let mut out = [0u64; 3];
        let status =
            unsafe { syscall6(SYS_SHARED_CREATE, POOL, 1, out.as_mut_ptr() as u64, 0, 0, 0) };
        if status != 0 {
            return Err(status);
        }
        let cap = out[0];
        let va = unsafe { syscall2(SYS_SHARED_MAP, cap, 1) };
        if va <= 0 {
            destroy(cap);
            return Err(va);
        }
        // SAFETY: this is a fresh, zeroed page with an exclusive trusted
        // owner mapping, so its canonical ring layout can be initialized.
        let initialized = unsafe {
            arena_runtime::streams::StreamSet::initialize(
                va as *mut u8,
                arena_runtime::streams::STREAM_PAGE_BYTES,
            )
        };
        if initialized.is_err() {
            // SAFETY: remove only the map returned above.
            let _ = unsafe { syscall6(SYS_SHARED_UNMAP, va as u64, 0, 0, 0, 0, 0) };
            destroy(cap);
            return Err(STATUS_BAD_ARG);
        }
        Ok(Self { cap, va: va as u64 })
    }

    fn commit(mut self) -> (u64, u64) {
        let pair = (self.cap, self.va);
        self.cap = CAP_NONE;
        self.va = 0;
        pair
    }
}

impl Drop for PendingStream {
    fn drop(&mut self) {
        if self.va != 0 {
            // SAFETY: this temporary owns exactly the map it received from
            // SYS_SHARED_MAP.
            let _ = unsafe { syscall6(SYS_SHARED_UNMAP, self.va, 0, 0, 0, 0, 0) };
        }
        if self.cap != CAP_NONE {
            destroy(self.cap);
        }
    }
}

static mut PREFS: arena_desktop::preferences::Preferences =
    arena_desktop::preferences::Preferences {
        dark: false,
        motion: true,
    };
static mut FILES: Option<arena_desktop::fs_backend::Fs> = None;
/// The broker's own filesd session (None = AFS2 offline or absent).
static mut AFS2: Option<arena_desktop::files::Files> = None;
/// Rights an application gets on a document it opened or saved.
const R_DOC: u8 = arena_desktop::filesd_wire::R_READ | arena_desktop::filesd_wire::R_WRITE;
static mut UPTIME_SECOND: u64 = 0;
static mut CAP_HIGH_WATER: u64 = 0;
static mut SHARED_CAP_BASELINE: usize = 0;
static mut NOTICE: Option<(&'static str, u64)> = None;
/// A built-in client's private clock was signalled for queued events and
/// it has not polled since (avoids re-signalling on every request).
static mut WOKEN: [bool; LIMIT] = [false; LIMIT];
/// Appearance changed: every built-in client should repaint now.
static mut WAKE_ALL: bool = false;
/// Badges on the compositor's own notification (ADR-0071). One wait
/// covers every event source; the merged badge needs no dispatch because
/// each wake re-runs sweep/animate/drain.
const BADGE_TIMER: u64 = 1;
/// A client or input request was queued while the compositor was not
/// parked in RECV (endpoint-bound notification).
const BADGE_REQUEST: u64 = 2;
/// A child application process exited (spawn exit notification).
const BADGE_EXIT: u64 = 4;
/// A peer may now read or write on the session's existing private clock.
const BADGE_STREAM_READY: u64 = 1 << 2;
/// filesd: the Desktop folder changed (directory watch, ADR-0079). A hint;
/// the broker confirms through its own record.
const BADGE_WATCH_BIT: u8 = 3;
const BADGE_WATCH: u64 = 1 << BADGE_WATCH_BIT;
static mut WATCH_SIGNALLED: bool = false;
/// Nothing time-driven is due: sleep until an event.
const NO_DEADLINE: u64 = u64::MAX;
/// Next instant time-driven presentation work is due: a running motion
/// (frame pacing), the next uptime second shown in the bar, or a notice
/// expiry. Everything else (requests, input, child exit) wakes the
/// compositor through its notification instead of a poll.
fn next_deadline(now: u64) -> u64 {
    let mut due = (now / 1_000_000 + 1) * 1_000_000;
    if let Some((_, until)) = unsafe { NOTICE } {
        due = due.min(until);
    }
    let sessions = unsafe { &*(&raw const SESSIONS) };
    if sessions
        .iter()
        .any(|s| s.id != 0 && (s.reveal.active(now) || s.focus.active(now) || s.ending))
        || unsafe { &*(&raw const EXTRA_WINDOWS) }
            .iter()
            .any(|w| w.handle != 0 && (w.reveal.active(now) || w.focus.active(now) || w.ending))
    {
        due = due.min(now + arena_ui::motion::FRAME_US);
    }
    // A held key repeats (window policy, Phase 11.4).
    if let Some(at) = unsafe { (*(&raw const WM)).repeat_deadline() } {
        due = due.min(at);
    }
    due
}
/// Signal live ABI-v2 clients that have queued events (or must repaint) on
/// the private clock the broker already holds for them. The native runtime's
/// `idle()` cancels a pending timer on an early notification wake.
fn wake_clients() {
    let state = unsafe { &*(&raw const WM) };
    let all = unsafe { core::mem::replace(&mut *(&raw mut WAKE_ALL), false) };
    for (i, s) in unsafe { &*(&raw const SESSIONS) }.iter().enumerate() {
        let extra_pending = unsafe { &*(&raw const EXTRA_WINDOWS) }
            .iter()
            .any(|w| w.handle != 0 && w.owner_session == i && state.pending(w.handle));
        if (s.handle == 0 && !extra_pending) || s.kind > 6 || unsafe { WOKEN[i] } {
            continue;
        }
        if (all || (s.handle != 0 && state.pending(s.handle)) || extra_pending)
            && unsafe { syscall2(SYS_NOTIFY, clock(i), 1) } == 0
        {
            unsafe { WOKEN[i] = true };
            if perf::ENABLED && unsafe { KEY_AT != 0 && !KEY_POLLED && KEY_NOTIFIED == 0 } {
                probe(P_KEY2NOTIFY, unsafe { KEY_AT });
                unsafe { KEY_NOTIFIED = perf_now() };
            }
        }
    }
}
// Opt-in latency probes (ARENA_PERF builds only; folded away otherwise).
use arena_desktop::perf::{self, Stat};
const P_RENDER: usize = 0;
const P_COMPOSE: usize = 1;
const P_RECTS: usize = 2;
const P_SKIPPED: usize = 3;
const P_PRESENT: usize = 4;
const P_DAMAGE: usize = 5;
const P_INPUT: usize = 6;
const P_SLEEP: usize = 7;
const P_REQUEST: usize = 8;
const P_KPIXELS: usize = 9;
const P_KEYDOWN: usize = 10;
const P_KEYUP: usize = 11;
const P_DELIVERED: usize = 12;
/// Key press received -> first frame presenting the client's answer.
const P_KEY2PHOTON: usize = 13;
/// Key press received -> the focused client's first poll / its Damage.
const P_KEY2POLL: usize = 14;
const P_KEY2DAMAGE: usize = 15;
/// The resource receipt logged after input-driven frames.
const P_SNAPSHOT: usize = 16;
/// Key press received -> its client notified; notified -> first poll.
const P_KEY2NOTIFY: usize = 17;
const P_NOTIFY2POLL: usize = 18;
/// Request handling split: decode/handle, reply, client wakes; the idle
/// tail from an empty TRY_RECV to WAIT.
const P_HANDLE: usize = 19;
const P_REPLY: usize = 20;
const P_WAKE: usize = 21;
const P_TAIL: usize = 22;
const P_COUNT: usize = 23;
static mut KEY_NOTIFIED: u64 = 0;
static mut KEY_POLLED: bool = false;
static mut KEY_AT: u64 = 0;
static mut KEY_FRAME: u64 = 0;
static mut PERF: [Stat; P_COUNT] = [Stat::ZERO; P_COUNT];
static mut PERF_LAST: u64 = 0;
fn probe(index: usize, since: u64) {
    if perf::ENABLED {
        let now = arena_desktop::app_client::now();
        unsafe { (*(&raw mut PERF))[index].add(now.saturating_sub(since)) };
    }
}
fn perf_now() -> u64 {
    if perf::ENABLED {
        arena_desktop::app_client::now()
    } else {
        0
    }
}
fn perf_report() {
    if !perf::ENABLED {
        return;
    }
    let now = arena_desktop::app_client::now();
    if now.saturating_sub(unsafe { PERF_LAST }) < 1_000_000 {
        return;
    }
    let stats = unsafe { *(&raw const PERF) };
    unsafe {
        PERF_LAST = now;
        PERF = [Stat::ZERO; P_COUNT];
    }
    let mut line = perf::Line::new();
    line.push(b"[perf desktop]");
    for (name, i) in [
        (&b"render"[..], P_RENDER),
        (b"compose", P_COMPOSE),
        (b"rects", P_RECTS),
        (b"nodamage", P_SKIPPED),
        (b"present", P_PRESENT),
        (b"damage", P_DAMAGE),
        (b"input2frame", P_INPUT),
        (b"sleep", P_SLEEP),
        (b"request", P_REQUEST),
        (b"kpx", P_KPIXELS),
        (b"keydown", P_KEYDOWN),
        (b"keyup", P_KEYUP),
        (b"delivered", P_DELIVERED),
        (b"key2photon", P_KEY2PHOTON),
        (b"key2poll", P_KEY2POLL),
        (b"key2damage", P_KEY2DAMAGE),
        (b"snapshot", P_SNAPSHOT),
        (b"key2notify", P_KEY2NOTIFY),
        (b"notify2poll", P_NOTIFY2POLL),
        (b"handle", P_HANDLE),
        (b"reply", P_REPLY),
        (b"wake", P_WAKE),
        (b"tail", P_TAIL),
    ] {
        line.stat(name, stats[i]);
    }
    line.push(b"\n");
    line.emit(log);
}
// Private broker memory: each session's snapshot region is mapped only by
// the broker; clients never map or receive it. Only an authenticated Damage
// or Resize request publishes into it.
fn snapshot_at(snapshot: u64) -> (&'static [u32], &'static [u32]) {
    let r = unsafe { RESERVE };
    if snapshot == 0 {
        return (&[], &[]);
    }
    let main = (r.surface_pages * 1024) as usize;
    unsafe {
        (
            core::slice::from_raw_parts(snapshot as *const u32, main),
            core::slice::from_raw_parts(
                (snapshot + r.surface_pages * 4096) as *const u32,
                (TRANSIENT_PAGES * 1024) as usize,
            ),
        )
    }
}
fn snapshot_of(s: &Session) -> (&'static [u32], &'static [u32]) {
    snapshot_at(s.snapshot)
}
fn snapshot_extra(s: &ExtraWindow) -> (&'static [u32], &'static [u32]) {
    snapshot_at(s.snapshot)
}
fn extra_index(handle: u64) -> Option<usize> {
    unsafe { &*(&raw const EXTRA_WINDOWS) }
        .iter()
        .position(|window| window.handle == handle)
}
/// Copy exactly the declared rectangles of a `stride`-wide raster from the
/// client's shared staging memory into the private snapshot. The sender is
/// blocked in CALL on this single-core topology; staging bytes outside the
/// rectangles never become visible.
fn publish_rects(src: u64, dst: u64, stride: usize, rects: &[[u16; 4]]) {
    for &[x, y, w, h] in rects {
        for row in y as usize..y as usize + h as usize {
            for col in x as usize..x as usize + w as usize {
                let pixel = row * stride + col;
                unsafe {
                    core::ptr::write_volatile(
                        (dst as *mut u32).add(pixel),
                        core::ptr::read_volatile((src as *const u32).add(pixel)),
                    );
                }
            }
        }
    }
}
static mut SESSIONS: [Session; LIMIT] = [EMPTY; LIMIT];
static mut EXTRA_WINDOWS: [ExtraWindow; EXTRA_LIMIT] = [EMPTY_EXTRA; EXTRA_LIMIT];
const ALL_APPS_CAPACITY: usize = 6 + arena_desktop::package::MAX_CATALOG_APPS;
static mut INSTALLED_APPS: [Option<arena_desktop::package::AppEntry>; ALL_APPS_CAPACITY] =
    [None; ALL_APPS_CAPACITY];
static mut INSTALLED_ASSOCIATIONS: [[[u8; 32]; 8]; ALL_APPS_CAPACITY] =
    [[[0; 32]; 8]; ALL_APPS_CAPACITY];
static mut INSTALLED_ASSOCIATION_COUNTS: [u8; ALL_APPS_CAPACITY] = [0; ALL_APPS_CAPACITY];
static mut INSTALLED_APP_COUNT: usize = 0;
static mut ALL_APPS_OPEN: bool = false;
static mut ALL_APPS_OPEN_WITH: bool = false;
static mut ALL_APPS_UNAVAILABLE: bool = false;
static mut ALL_APPS_QUERY: [u8; 32] = [0; 32];
static mut ALL_APPS_QUERY_LEN: usize = 0;
static mut ALL_APPS_SELECTED: usize = 0;
static mut ALL_APPS_TOP: usize = 0;
static mut ALL_APPS_BUTTONS: u8 = 0;
static mut OPEN_WITH_DOCUMENT: u64 = CAP_NONE;
static mut OPEN_WITH_OWNER: usize = usize::MAX;
static mut OPEN_WITH_TYPE: [u8; 32] = [0; 32];
static mut OPEN_WITH_TITLE: [u8; 32] = [0; 32];
static mut OPEN_WITH_READ_ONLY: bool = false;
static mut ASSOCIATION_DEFAULTS: arena_desktop::associations::Defaults =
    arena_desktop::associations::Defaults::new();
/// Badges are unique for this Desktop endpoint lifetime. Zero remains the
/// kernel's plain-endpoint marker; exhaustion refuses rather than wrapping.
static mut NEXT_SESSION_BADGE: u32 = 1;
/// The desktop owns every application Process cap through this generation-safe
/// group; Session stores only a local handle, never a PID or raw cap slot.
/// AppInstance slot -> its ProcessGroup. The slot is descriptive routing only;
/// operations still use the generation-checked group handle and held Process
/// cap. Keeping groups per instance prevents one app from consuming or
/// reaping another app's members.
static mut APP_GROUPS: [Option<ProcessGroup<ChildProcess, APP_GROUP_PROCESSES>>; LIMIT] =
    [const { None }; LIMIT];
static mut WM: State = match State::new(800, 600) {
    Ok(s) => s,
    Err(_) => panic!("constant geometry"),
};
#[panic_handler]
fn panic(_: &PanicInfo<'_>) -> ! {
    die(99)
}
fn die(c: u64) -> ! {
    let digits = [b'0' + ((c / 10) % 10) as u8, b'0' + (c % 10) as u8];
    log(b"[desktop] failed stage ");
    log(&digits);
    log(b"\n");
    arena_desktop::client::exit(c)
}
fn log(s: &[u8]) {
    unsafe {
        syscall2(SYS_DEBUG_WRITE, s.as_ptr() as u64, s.len() as u64);
    }
}
fn log_launch_refusal(stage: &[u8], status: i64) {
    log(b"[desktop] launch-refusal stage/status=");
    log(stage);
    log(b"/");
    log_number(status as u64);
    log(b"\n");
}
fn log_number(mut value: u64) {
    let mut digits = [0u8; 20];
    let mut n = 0;
    loop {
        digits[n] = b'0' + (value % 10) as u8;
        n += 1;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    for digit in digits[..n].iter().rev() {
        log(core::slice::from_ref(digit));
    }
}
/// Sample native occupancy while request references are still landed.
fn transient_caps() {
    let mut counts = [0u64; 9];
    if unsafe { syscall6(SYS_OBSERVE, POOL, counts.as_mut_ptr() as u64, 0, 0, 0, 0) } != 0 {
        die(77)
    }
    if counts[7] > unsafe { CAP_HIGH_WATER } {
        unsafe { CAP_HIGH_WATER = counts[7] };
        log(b"[desktop] measured broker cap high-water=");
        log_number(counts[7]);
        log(b"\n");
    }
}
type ResourceReceipt = [u64; 7];
/// Sample all seven fields used by the Desktop's resource receipts.
fn observe_receipt() -> ResourceReceipt {
    let mut counts = [0u64; 9];
    if unsafe { syscall6(SYS_OBSERVE, POOL, counts.as_mut_ptr() as u64, 0, 0, 0, 0) } != 0 {
        die(77)
    }
    [
        counts[0], counts[2], counts[3], counts[4], counts[5], counts[6], counts[7],
    ]
}
fn log_resource_receipt(prefix: &[u8], receipt: ResourceReceipt) {
    let mut line = perf::Line::new();
    line.push(prefix);
    for (i, value) in receipt.iter().enumerate() {
        if i != 0 {
            line.push(b"/");
        }
        line.number(*value);
    }
    line.push(b"\n");
    log(line.as_bytes());
}
/// The last resource receipt logged (Phase 11 latency: an unchanged
/// receipt after every pointer frame cost milliseconds of serial output).
static mut LAST_RECEIPT: Option<ResourceReceipt> = None;
/// Log the measured resource receipt. Lifecycle events (`force`) always
/// log, so a refusal is proven by a fresh unchanged receipt; frames log
/// only a receipt that differs from the last one, so every change (and
/// every peak) is still recorded.
fn snapshot(force: bool) {
    let receipt = observe_receipt();
    if !force && unsafe { LAST_RECEIPT } == Some(receipt) {
        return;
    }
    unsafe { LAST_RECEIPT = Some(receipt) };
    log_resource_receipt(
        b"[desktop] measured frames/records/processes/regions/pages/maps/caps=",
        receipt,
    );
}

// ---- the trusted chooser (powerbox, ADR-0077) ------------------------------
//
// Drawn by the shell, driven only by the user's own input, resolving names
// through the broker's own filesd page on /Users/user. The requesting
// application learns only the outcome and receives a capability for
// exactly the chosen file, in its own lineage.
const CHOOSER_MAX: usize = 64;
struct Chooser {
    session: usize,
    save: bool,
    read_only: bool,
    /// Folder shown, relative to /Users/user.
    dir: [u8; 200],
    dir_len: usize,
    dir_cap: u64,
    entries: [arena_desktop::files::Entry; CHOOSER_MAX],
    count: usize,
    more: bool,
    selected: Option<usize>,
    top: usize,
    name: [u8; 32],
    name_len: usize,
    /// Save over an existing file asks once more.
    confirm: bool,
    message: &'static str,
    buttons: u8,
}
static mut CHOOSER: Option<Chooser> = None;

fn chooser_open(index: usize, save: bool, read_only: bool, name: [u8; 32]) -> Result<(), i64> {
    let s = unsafe { SESSIONS[index] };
    if unsafe { (*(&raw const CHOOSER)).is_some() } || s.handle == 0 || s.grant_ready {
        return Err(STATUS_BUSY);
    }
    if unsafe { (*(&raw const AFS2)).is_none() } || s.files_head == CAP_NONE {
        return Err(-3015);
    }
    let n = name.iter().position(|b| *b == 0).unwrap_or(32).min(31);
    let mut ch = Chooser {
        session: index,
        save,
        read_only,
        dir: [0; 200],
        dir_len: 9,
        dir_cap: CAP_NONE,
        entries: [arena_desktop::files::Entry::EMPTY; CHOOSER_MAX],
        count: 0,
        more: false,
        selected: None,
        top: 0,
        name: [0; 32],
        name_len: if save { n } else { 0 },
        confirm: false,
        message: "",
        buttons: 0,
    };
    ch.dir[..9].copy_from_slice(b"Documents");
    if save {
        ch.name[..n].copy_from_slice(&name[..n]);
    }
    unsafe { CHOOSER = Some(ch) };
    chooser_load();
    log(b"[desktop] trusted chooser opened\n");
    Ok(())
}
fn chooser_close() {
    if let Some(ch) = unsafe { (*(&raw mut CHOOSER)).take() } {
        if ch.dir_cap != CAP_NONE {
            if let Some(f) = unsafe { &*(&raw const AFS2) } {
                f.release(ch.dir_cap);
            }
        }
    }
}
/// (Re)list the chooser's folder through the broker's own session.
fn chooser_load() {
    let Some(ch) = (unsafe { (*(&raw mut CHOOSER)).as_mut() }) else {
        return;
    };
    let Some(f) = (unsafe { &*(&raw const AFS2) }) else {
        return;
    };
    if ch.dir_cap != CAP_NONE {
        f.release(ch.dir_cap);
        ch.dir_cap = CAP_NONE;
    }
    use arena_desktop::filesd_wire::{R_CREATE, R_LIST};
    ch.count = 0;
    ch.selected = None;
    ch.top = 0;
    // Rights only narrow from a folder to what is opened in it, so the
    // folder carries what the chooser may grant (R_DOC) as well.
    match f.walk(USER_ROOT, &ch.dir[..ch.dir_len], R_LIST | R_CREATE | R_DOC) {
        Ok((cap, 2)) => ch.dir_cap = cap,
        Ok((cap, _)) => {
            f.release(cap);
            ch.message = "NOT A FOLDER";
            return;
        }
        Err(e) => {
            ch.message = arena_desktop::files::describe_status(e);
            return;
        }
    }
    let mut after = [0u8; arena_desktop::files::NAME_MAX];
    let mut after_len = 0;
    ch.more = false;
    while ch.count < CHOOSER_MAX {
        match f.list(ch.dir_cap, &after[..after_len], &mut ch.entries[ch.count..]) {
            Ok((0, _)) => break,
            Ok((n, more)) => {
                ch.count += n;
                let last = ch.entries[ch.count - 1];
                after[..last.name().len()].copy_from_slice(last.name());
                after_len = last.name().len();
                if !more {
                    break;
                }
                ch.more = ch.count == CHOOSER_MAX;
            }
            Err(e) => {
                ch.message = arena_desktop::files::describe_status(e);
                break;
            }
        }
    }
    if ch.more {
        ch.message = "SHOWING THE FIRST 64 ITEMS";
    }
}
/// Finish: hand the outcome to the requesting session and close.
fn chooser_finish(granted: Option<(u64, [u8; 32])>) {
    let Some(index) = (unsafe { (*(&raw const CHOOSER)).as_ref() }).map(|c| c.session) else {
        return;
    };
    let (save, read_only) = unsafe { (*(&raw const CHOOSER)).as_ref() }
        .map_or((false, false), |c| (c.save, c.read_only));
    chooser_close();
    let s = unsafe { &mut *(&raw mut SESSIONS).cast::<Session>().add(index) };
    s.grant_ready = true;
    s.grant_save = save;
    s.grant_read_only = read_only;
    match granted {
        Some((cap, title)) => {
            s.grant = cap;
            s.grant_title = title;
            log(b"[desktop] trusted chooser granted one file capability\n");
        }
        None => {
            s.grant = CAP_NONE;
            log(b"[desktop] trusted chooser cancelled; nothing granted\n");
        }
    }
    // Delivered by the main loop once no window-policy borrow is live.
    unsafe { CHOSEN_TO = s.handle };
}
/// The window whose chooser just finished (0: none).
static mut CHOSEN_TO: u64 = 0;
fn deliver_chosen() {
    let handle = unsafe { core::mem::replace(&mut *(&raw mut CHOSEN_TO), 0) };
    if handle != 0 {
        unsafe { (&mut *(&raw mut WM)).send(handle, arena_desktop::model::Event::Chosen) };
    }
}
fn chooser_enter(name: &[u8]) {
    let Some(ch) = (unsafe { (*(&raw mut CHOOSER)).as_mut() }) else {
        return;
    };
    let extra = name.len() + usize::from(ch.dir_len != 0);
    if ch.dir_len + extra > ch.dir.len() {
        ch.message = "PATH TOO LONG";
        return;
    }
    if ch.dir_len != 0 {
        ch.dir[ch.dir_len] = b'/';
        ch.dir_len += 1;
    }
    ch.dir[ch.dir_len..ch.dir_len + name.len()].copy_from_slice(name);
    ch.dir_len += name.len();
    ch.message = "";
    chooser_load();
}
fn chooser_up() {
    let Some(ch) = (unsafe { (*(&raw mut CHOOSER)).as_mut() }) else {
        return;
    };
    if ch.dir_len == 0 {
        return;
    }
    ch.dir_len = ch.dir[..ch.dir_len]
        .iter()
        .rposition(|b| *b == b'/')
        .unwrap_or(0);
    ch.message = "";
    chooser_load();
}
/// Open the selected file, or save under the typed name.
fn chooser_accept() {
    let Some(ch) = (unsafe { (*(&raw mut CHOOSER)).as_mut() }) else {
        return;
    };
    let Some(f) = (unsafe { &*(&raw const AFS2) }) else {
        return;
    };
    let head = unsafe { SESSIONS[ch.session].files_head };
    let mut title = [0u8; 32];
    if !ch.save {
        let Some(i) = ch.selected else {
            ch.message = "SELECT A DOCUMENT";
            return;
        };
        let e = ch.entries[i];
        if e.is_dir() {
            return chooser_enter(e.name());
        }
        let n = e.name().len().min(31);
        title[..n].copy_from_slice(&e.name()[..n]);
        let rights = if ch.read_only {
            arena_desktop::filesd_wire::R_READ
        } else {
            R_DOC
        };
        match f.open_child_in(ch.dir_cap, e.name(), rights, head) {
            Ok((cap, _)) => chooser_finish(Some((cap, title))),
            Err(e) => ch.message = arena_desktop::files::describe_status(e),
        }
        return;
    }
    let name = &ch.name[..ch.name_len];
    if name.is_empty() {
        ch.message = "TYPE A NAME";
        return;
    }
    let exists = ch.entries[..ch.count].iter().any(|e| e.name() == name);
    if exists
        && ch.entries[..ch.count]
            .iter()
            .any(|e| e.name() == name && e.is_dir())
    {
        ch.message = "A FOLDER HAS THAT NAME";
        return;
    }
    if exists && !ch.confirm {
        ch.confirm = true;
        ch.message = "REPLACE THE EXISTING FILE? SAVE AGAIN";
        return;
    }
    if !exists {
        if let Err(e) = f.create(ch.dir_cap, name) {
            ch.message = arena_desktop::files::describe_status(e);
            return;
        }
    }
    title[..name.len()].copy_from_slice(name);
    match f.open_child_in(ch.dir_cap, name, R_DOC, head) {
        Ok((cap, _)) => chooser_finish(Some((cap, title))),
        Err(e) => ch.message = arena_desktop::files::describe_status(e),
    }
}
fn chooser_select(i: usize) {
    let Some(ch) = (unsafe { (*(&raw mut CHOOSER)).as_mut() }) else {
        return;
    };
    if i >= ch.count {
        return;
    }
    ch.selected = Some(i);
    if i < ch.top {
        ch.top = i;
    }
    if i >= ch.top + arena_desktop::shell::CHOOSER_ROWS {
        ch.top = i + 1 - arena_desktop::shell::CHOOSER_ROWS;
    }
    let e = ch.entries[i];
    if ch.save && !e.is_dir() && e.name().len() < 32 {
        ch.name = [0; 32];
        ch.name[..e.name().len()].copy_from_slice(e.name());
        ch.name_len = e.name().len();
        ch.confirm = false;
    }
}
/// A key while the chooser is open. All keys go to the chooser (modal).
fn chooser_key(code: u16) {
    let Some(ch) = (unsafe { (*(&raw mut CHOOSER)).as_mut() }) else {
        return;
    };
    match code {
        27 => chooser_finish(None),
        258 => {
            let i = ch.selected.map_or(0, |i| i.saturating_sub(1));
            chooser_select(i)
        }
        259 => {
            let i = ch
                .selected
                .map_or(0, |i| i + 1)
                .min(ch.count.saturating_sub(1));
            chooser_select(i)
        }
        13 => match ch.selected.map(|i| ch.entries[i]) {
            Some(e) if e.is_dir() => chooser_enter(e.name()),
            _ => chooser_accept(),
        },
        // Backspace edits the name while saving (it never leaves the
        // folder behind the user's back); Left goes up in either mode.
        8 if ch.save => {
            if ch.name_len > 0 {
                ch.name_len -= 1;
                ch.name[ch.name_len] = 0;
                ch.confirm = false;
            }
        }
        8 | 256 => chooser_up(),
        32..=126 if ch.save && ch.name_len < 31 && code != u16::from(b'/') => {
            ch.name[ch.name_len] = code as u8;
            ch.name_len += 1;
            ch.confirm = false;
        }
        _ => {}
    }
}
/// The pointer while the chooser is open: presses go to the chooser only.
fn chooser_pointer(x: i32, y: i32, buttons: u8, w: i32, h: i32) {
    use arena_desktop::shell::{ChooserHit as H, chooser_hit};
    let Some(ch) = (unsafe { (*(&raw mut CHOOSER)).as_mut() }) else {
        return;
    };
    let pressed = buttons & 1 != 0 && ch.buttons & 1 == 0;
    ch.buttons = buttons;
    if !pressed {
        return;
    }
    match chooser_hit(w, h, x, y) {
        H::Row(r) => {
            let i = ch.top + r;
            if ch.selected == Some(i) {
                // A second press on the selected row opens it.
                chooser_key(13);
            } else {
                chooser_select(i);
            }
        }
        H::Up => chooser_up(),
        H::Cancel => chooser_finish(None),
        H::Accept => chooser_accept(),
        _ => {}
    }
}
fn chooser_view() -> Option<arena_desktop::shell::ChooserView> {
    let ch = unsafe { (*(&raw const CHOOSER)).as_ref() }?;
    let rows_max = arena_desktop::shell::CHOOSER_ROWS;
    let mut v = arena_desktop::shell::ChooserView {
        save: ch.save,
        read_only: ch.read_only,
        place: [0; 48],
        rows: [[0; 32]; arena_desktop::shell::CHOOSER_ROWS],
        count: 0,
        selected: None,
        above: ch.top > 0,
        below: ch.top + rows_max < ch.count,
        name: ch.name,
        message: [0; 48],
    };
    let mut k = 0;
    for part in [
        b"Home" as &[u8],
        if ch.dir_len > 0 { b"/" } else { b"" },
        &ch.dir[..ch.dir_len],
    ] {
        let n = part.len().min(47 - k);
        v.place[k..k + n].copy_from_slice(&part[..n]);
        k += n;
    }
    let m = ch.message.as_bytes();
    let n = m.len().min(47);
    v.message[..n].copy_from_slice(&m[..n]);
    for (row, e) in ch.entries[ch.top..ch.count]
        .iter()
        .take(rows_max)
        .enumerate()
    {
        let name = e.name();
        let room = if e.is_dir() { 30 } else { 31 };
        let n = name.len().min(room);
        v.rows[row][..n].copy_from_slice(&name[..n]);
        if name.len() > room {
            v.rows[row][n - 1] = b'~';
        }
        if e.is_dir() {
            v.rows[row][n] = b'/';
        }
        v.count += 1;
    }
    v.selected = ch
        .selected
        .filter(|i| *i >= ch.top && *i < ch.top + rows_max)
        .map(|i| (i - ch.top) as u8);
    Some(v)
}

/// The broker's filesd session: one page of its own region (ADR-0077).
/// With the service offline (no AFS2 region) nothing is created.
fn start_afs2() {
    use arena_desktop::files::Files;
    if describe(USER_ROOT).is_none() || !Files::online(USER_ROOT) {
        log(b"[desktop] AFS2 file service unavailable; file capabilities offline\n");
        return;
    }
    let mut out = [0; 3];
    if unsafe { syscall6(SYS_SHARED_CREATE, POOL, 1, out.as_mut_ptr() as u64, 0, 0, 0) } != 0 {
        die(76)
    }
    let va = unsafe { syscall2(SYS_SHARED_MAP, out[0], 1) };
    if va <= 0 {
        die(76)
    }
    let session = Files::session(USER_ROOT, out[0], va as u64, 0);
    // The broker keeps only its mapping; filesd holds its own.
    destroy(out[0]);
    match session {
        Ok(files) => {
            unsafe { AFS2 = Some(files) };
            log(b"[desktop] AFS2 file service online; /Users/user granted per session\n");
        }
        Err(_) => {
            unsafe { syscall6(SYS_SHARED_UNMAP, va as u64, 0, 0, 0, 0, 0) };
            log(b"[desktop] AFS2 session refused; file capabilities offline\n");
        }
    }
}

const ASSOCIATION_FILE: &[u8] = b".arena-app-associations";

fn load_association_defaults() {
    let Some(files) = (unsafe { *(&raw const AFS2) }) else {
        return;
    };
    let Ok((file, _)) = files.walk(
        USER_ROOT,
        ASSOCIATION_FILE,
        arena_desktop::filesd_wire::R_READ,
    ) else {
        return;
    };
    let mut bytes = [0u8; arena_desktop::associations::BYTES];
    if let Ok(record) = files.read_all(file, &mut bytes)
        && let Some(defaults) = arena_desktop::associations::Defaults::decode(record)
    {
        unsafe { ASSOCIATION_DEFAULTS = defaults };
        log(b"[desktop] AFS2 application handler defaults loaded\n");
    }
    files.release(file);
}

fn save_association_defaults() -> Result<(), u64> {
    let Some(files) = (unsafe { *(&raw const AFS2) }) else {
        return Err(arena_desktop::filesd_wire::S_OFFLINE);
    };
    let file = match files.walk(
        USER_ROOT,
        ASSOCIATION_FILE,
        arena_desktop::filesd_wire::R_READ | arena_desktop::filesd_wire::R_WRITE,
    ) {
        Ok((file, _)) => file,
        Err(arena_desktop::filesd_wire::S_NOENT) => {
            files.create(USER_ROOT, ASSOCIATION_FILE)?;
            files
                .walk(
                    USER_ROOT,
                    ASSOCIATION_FILE,
                    arena_desktop::filesd_wire::R_READ | arena_desktop::filesd_wire::R_WRITE,
                )?
                .0
        }
        Err(error) => return Err(error),
    };
    let bytes = unsafe { &*(&raw const ASSOCIATION_DEFAULTS) }.encode();
    let result = files.write_all(file, &bytes);
    files.release(file);
    result
}

fn builtin_associations(
    kind: usize,
) -> (
    [[u8; arena_desktop::package::manifest::CONTENT_TYPE_BYTES];
        arena_desktop::package::manifest::MAX_ASSOCIATIONS],
    u8,
) {
    let mut associations = [[0; arena_desktop::package::manifest::CONTENT_TYPE_BYTES];
        arena_desktop::package::manifest::MAX_ASSOCIATIONS];
    if kind == arena_desktop::apps::EDITOR as usize {
        associations[0][..10].copy_from_slice(b"text/plain");
        associations[1][..13].copy_from_slice(b"text/markdown");
    }
    let count = if kind == arena_desktop::apps::EDITOR as usize {
        2
    } else {
        0
    };
    (associations, count)
}

fn app_handles_type(index: usize, content_type: &[u8]) -> bool {
    let counts = unsafe { &*(&raw const INSTALLED_ASSOCIATION_COUNTS) };
    let associations = unsafe { &*(&raw const INSTALLED_ASSOCIATIONS) };
    associations[index][..usize::from(counts[index])]
        .iter()
        .any(|field| {
            field.iter().position(|byte| *byte == 0).is_some_and(|end| {
                field[..end] == *content_type && field[end..].iter().all(|byte| *byte == 0)
            })
        })
}

fn content_type_for_name(name: &[u8; 32]) -> &'static [u8] {
    let end = name.iter().position(|byte| *byte == 0).unwrap_or(32);
    let name = &name[..end];
    if name.len() >= 3 && name[name.len() - 3..].eq_ignore_ascii_case(b".md") {
        b"text/markdown"
    } else if name.len() >= 4
        && [b".txt".as_slice(), b".log".as_slice()]
            .iter()
            .any(|suffix| name[name.len() - suffix.len()..].eq_ignore_ascii_case(suffix))
    {
        b"text/plain"
    } else if !name.contains(&b'.') {
        b"text/plain"
    } else {
        b"application/octet-stream"
    }
}

fn any_handler(content_type: &[u8]) -> bool {
    unsafe { &*(&raw const INSTALLED_APPS) }
        .iter()
        .take(unsafe { INSTALLED_APP_COUNT })
        .enumerate()
        .any(|(index, entry)| entry.is_some() && app_handles_type(index, content_type))
}

fn handler_index(application_id: &[u8; 32], content_type: &[u8]) -> Option<usize> {
    unsafe { &*(&raw const INSTALLED_APPS) }
        .iter()
        .take(unsafe { INSTALLED_APP_COUNT })
        .position(|entry| entry.is_some_and(|entry| entry.application_id == *application_id))
        .filter(|index| app_handles_type(*index, content_type))
}

fn handler_count(content_type: &[u8]) -> usize {
    unsafe { &*(&raw const INSTALLED_APPS) }
        .iter()
        .take(unsafe { INSTALLED_APP_COUNT })
        .enumerate()
        .filter(|(index, entry)| entry.is_some() && app_handles_type(*index, content_type))
        .count()
}

fn open_document_with_registry(
    document: u64,
    title: [u8; 32],
    force_chooser: bool,
    owner: usize,
    read_only: bool,
) {
    if document == CAP_NONE {
        return;
    }
    if unsafe { OPEN_WITH_DOCUMENT != CAP_NONE } {
        release_file_cap(document);
        desk_notice("FINISH THE CURRENT OPEN WITH REQUEST FIRST");
        return;
    }
    let content_type = content_type_for_name(&title);
    if !any_handler(content_type) {
        release_file_cap(document);
        desk_notice("NO INSTALLED APPLICATION CAN OPEN THIS FILE");
        return;
    }
    if !force_chooser {
        let default = unsafe { &*(&raw const ASSOCIATION_DEFAULTS) }
            .get(content_type)
            .copied()
            .and_then(|id| handler_index(&id, content_type));
        if let Some(index) = default {
            let _ = launch_registry_entry_with_title(index, document, title, read_only);
            return;
        }
        if handler_count(content_type) == 1 {
            let index = unsafe { &*(&raw const INSTALLED_APPS) }
                .iter()
                .take(unsafe { INSTALLED_APP_COUNT })
                .enumerate()
                .position(|(index, entry)| entry.is_some() && app_handles_type(index, content_type))
                .unwrap_or(0);
            let _ = launch_registry_entry_with_title(index, document, title, read_only);
            return;
        }
    }
    unsafe { OPEN_WITH_TITLE = title };
    open_with_document(document, owner, title, read_only);
}

fn launch_registry_entry_with_title(
    index: usize,
    document: u64,
    title: [u8; 32],
    read_only: bool,
) -> Result<(), i64> {
    let app = unsafe { (&*(&raw const INSTALLED_APPS))[index] }.ok_or(PKG_STALE as i64)?;
    let result = if app.builtin_kind != u8::MAX {
        launch_with_document(app.builtin_kind, title, document, read_only)
    } else {
        launch_installed_application(&app.application_id, app.flags, document, read_only)
    };
    release_file_cap(document);
    if result.is_err() {
        desk_notice("APPLICATION LAUNCH REFUSED");
    }
    result
}

fn save_selected_handler_default() {
    if !unsafe { ALL_APPS_OPEN_WITH } {
        return;
    }
    let Some(index) = filtered_application_index(unsafe { ALL_APPS_SELECTED }) else {
        return;
    };
    let entry = unsafe { (&*(&raw const INSTALLED_APPS))[index] }.unwrap_or_else(|| die(97));
    let content_type = unsafe { &*(&raw const OPEN_WITH_TYPE) };
    let len = content_type
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(32);
    let prior = unsafe { &*(&raw const ASSOCIATION_DEFAULTS) }.encode();
    if unsafe { &mut *(&raw mut ASSOCIATION_DEFAULTS) }
        .set(&content_type[..len], &entry.application_id)
        .is_err()
    {
        desk_notice("HANDLER DEFAULT LIMIT REACHED");
        return;
    }
    if save_association_defaults().is_ok() {
        desk_notice("DEFAULT APPLICATION SAVED");
        log(b"[desktop] AFS2 application handler default saved; no File cap granted\n");
    } else {
        if let Some(restored) = arena_desktop::associations::Defaults::decode(&prior) {
            unsafe { ASSOCIATION_DEFAULTS = restored };
        }
        desk_notice("COULD NOT SAVE HANDLER DEFAULT");
    }
}

fn prune_association_defaults() {
    let entries = unsafe { &*(&raw const INSTALLED_APPS) };
    let count = unsafe { INSTALLED_APP_COUNT };
    let removed = unsafe { &mut *(&raw mut ASSOCIATION_DEFAULTS) }.retain(|content_type, id| {
        entries
            .iter()
            .take(count)
            .enumerate()
            .any(|(index, entry)| {
                entry.is_some_and(|entry| entry.application_id == *id)
                    && app_handles_type(index, content_type)
            })
    });
    if removed > 0 {
        let _ = save_association_defaults();
        log(b"[desktop] removed stale application handler defaults\n");
    }
}
// ---- the desktop surface (Phase 11.8) ---------------------------------------
//
// /Users/user/Desktop as icons on the background, resolved through the
// broker's own capability for /Users/user (the same one the chooser uses).
// Opening a document grants the new Editor exactly that file, in its own
// lineage, as a Files offer does.
static mut DESK: arena_desktop::desk::Desk = arena_desktop::desk::Desk::new();
/// The broker's record for /Users/user/Desktop, watched (or CAP_NONE).
static mut DESK_WATCH: u64 = CAP_NONE;
static mut DESK_BUTTONS: u8 = 0;
fn desk_store() -> Option<arena_desktop::apps::explorer::CapStore> {
    unsafe { *(&raw const AFS2) }.map(|fs| arena_desktop::apps::explorer::CapStore {
        fs,
        root: USER_ROOT,
    })
}
fn desk_load(w: i32, h: i32) {
    if let Some(mut store) = desk_store() {
        let desk = unsafe { &mut *(&raw mut DESK) };
        desk.screen = (w, h);
        if desk.load(&mut store).is_ok() {
            log(b"[desktop] desktop surface shows /Users/user/Desktop\n");
        }
        desk_watch();
    }
}
/// Watch the Desktop folder through the broker's own record for it
/// (a capability, ADR-0079); its badge lands on the broker's clock.
fn desk_watch() {
    let Some(files) = (unsafe { *(&raw const AFS2) }) else {
        return;
    };
    let old = unsafe { core::mem::replace(&mut *(&raw mut DESK_WATCH), CAP_NONE) };
    if old != CAP_NONE {
        files.release(old);
    }
    let Ok((cap, _)) = files.walk(USER_ROOT, b"Desktop", arena_desktop::filesd_wire::R_LIST) else {
        return;
    };
    if files.watch(cap, CLOCK, BADGE_WATCH_BIT).is_ok() {
        unsafe { DESK_WATCH = cap };
        log(b"[desktop] watching /Users/user/Desktop\n");
    } else {
        files.release(cap);
        log(b"[desktop] Desktop watch refused\n");
    }
}
fn desk_notice(text: &'static str) {
    unsafe {
        NOTICE = Some((text, arena_desktop::app_client::now() + 3_000_000));
    }
}
fn desk_effect(e: arena_desktop::desk::Effect) {
    use arena_desktop::desk::Effect;
    match e {
        Effect::None => {}
        Effect::NoApplication => desk_notice("NO APPLICATION CAN OPEN THIS FILE"),
        Effect::OpenFolder(p) => {
            // Where Files starts, inside the home capability it is granted
            // anyway: presentation, not authority.
            let mut path = [0u8; 32];
            let b = p.bytes();
            let fits = b.len() <= 32 && b.iter().all(|c| c.is_ascii_graphic() || *c == b' ');
            let shown: &[u8] = if fits { b } else { b"Desktop" };
            path[..shown.len()].copy_from_slice(shown);
            if launch(1, path, CAP_NONE).is_err() {
                desk_notice("LAUNCH REFUSED / DESKTOP CAPACITY");
            }
        }
        Effect::InstallBundle(p) => {
            let Some(files) = (unsafe { *(&raw const AFS2) }) else {
                desk_notice("FILESYSTEM SERVICE OFFLINE");
                return;
            };
            if arena_desktop::files::apb1_probe(USER_ROOT)
                != Ok(arena_desktop::filesd_wire::S_DENIED)
            {
                log(b"[desktop] APB1 authority boundary FAILED: ordinary Filesd capability not denied\n");
                desk_notice("PACKAGE AUTHORITY CHECK FAILED");
                return;
            }
            log(b"[desktop] APB1 authority boundary: ordinary Filesd capability denied protected install\n");
            // The suffix only selects this affordance after the user's
            // explicit open action. Open the source via the broker's exact
            // read-only filesd capability; receiver-side APKG/signature and
            // transaction checks are the only install authorization.
            match files.walk(USER_ROOT, p.bytes(), arena_desktop::filesd_wire::R_READ) {
                Ok((source, _)) => {
                    let result = arena_desktop::package::install_apb1(source);
                    files.release(source);
                    match result {
                        Ok(receipt) => {
                            log(b"[desktop] APB1 installed; signed version=");
                            log_number(receipt.version);
                            log(b" (no launch authority implied)\n");
                            let _ = refresh_installed_applications();
                            desk_notice("APPLICATION INSTALLED");
                        }
                        Err(status) if status == PKG_OFFLINE => {
                            desk_notice("PACKAGE SERVICE OFFLINE");
                        }
                        Err(status) if status == PKG_NO_SPACE => {
                            desk_notice("NOT ENOUGH AFS2 SPACE");
                        }
                        Err(status) if status == PKG_CONFLICT => {
                            desk_notice("APPLICATION VERSION ALREADY INSTALLED");
                        }
                        Err(status) if status == PKG_DOWNGRADE => {
                            desk_notice("PACKAGE VERSION REFUSED BY POLICY");
                        }
                        Err(_) => desk_notice("SIGNED PACKAGE REFUSED"),
                    }
                }
                Err(_) => desk_notice("THE PACKAGE FILE COULD NOT BE OPENED"),
            }
        }
        Effect::OpenDocument(p) => {
            let Some(files) = (unsafe { *(&raw const AFS2) }) else {
                return;
            };
            match files.walk(USER_ROOT, p.bytes(), R_DOC) {
                Ok((doc, _)) => {
                    let mut title = [0u8; 32];
                    for (t, c) in title.iter_mut().zip(p.name()) {
                        *t = if c.is_ascii_graphic() || *c == b' ' {
                            *c
                        } else {
                            b'?'
                        };
                    }
                    open_document_with_registry(doc, title, false, usize::MAX, false);
                }
                Err(_) => desk_notice("THE FILE COULD NOT BE OPENED"),
            }
        }
        Effect::OpenWithDocument(p) => {
            let Some(files) = (unsafe { *(&raw const AFS2) }) else {
                return;
            };
            match files.walk(USER_ROOT, p.bytes(), arena_desktop::filesd_wire::R_READ) {
                Ok((doc, _)) => {
                    let mut title = [0u8; 32];
                    for (t, c) in title.iter_mut().zip(p.name()) {
                        *t = if c.is_ascii_graphic() || *c == b' ' {
                            *c
                        } else {
                            b'?'
                        };
                    }
                    open_document_with_registry(doc, title, true, usize::MAX, true);
                }
                Err(_) => desk_notice("THE FILE COULD NOT BE OPENED"),
            }
        }
    }
}
/// The Desktop watch fired: confirm through the broker's own record, then
/// re-list (a folder that vanished is looked up and watched again).
fn desk_watched() {
    if !unsafe { core::mem::replace(&mut *(&raw mut WATCH_SIGNALLED), false) } {
        return;
    }
    let (Some(files), Some(mut store)) = (unsafe { *(&raw const AFS2) }, desk_store()) else {
        return;
    };
    let cap = unsafe { DESK_WATCH };
    match files.watched(cap) {
        Ok((0, false)) => {}
        Ok((_, false)) => {
            unsafe { (*(&raw mut DESK)).poll(&mut store) };
        }
        _ => {
            desk_watch();
            let _ = unsafe { (*(&raw mut DESK)).load(&mut store) };
        }
    }
}

fn describe(slot: u64) -> Option<[u64; 3]> {
    let mut d = [0; 3];
    (slot != CAP_NONE && unsafe { syscall2(SYS_CAP_DESCRIBE, slot, d.as_mut_ptr() as u64) } == 0)
        .then_some(d)
}
/// Descriptors retained outside the small kernel stack across a full-session
/// refusal. The summary is deliberately small; equality is checked slotwise.
static mut REFUSAL_CAPS: [[u64; 3]; CAP_SLOTS] = [[0; 3]; CAP_SLOTS];
#[derive(Clone, Copy)]
struct CapInventory {
    occupied: u64,
    digest: u64,
}
fn cap_slot_descriptor(slot: usize) -> [u64; 3] {
    match unsafe { syscall6(SYS_CAP_OCCUPIED, slot as u64, 0, 0, 0, 0, 0) } {
        0 => [0; 3],
        1 => describe(slot as u64).unwrap_or_else(|| die(97)),
        _ => die(97),
    }
}
fn inventory_digest_add(digest: &mut u64, value: u64) {
    let mut bytes = value;
    for _ in 0..8 {
        *digest ^= bytes & 0xff;
        *digest = digest.wrapping_mul(0x0000_0100_0000_01b3);
        bytes >>= 8;
    }
}
fn held_cap_kind_count(kind: u64) -> usize {
    (0..CAP_SLOTS)
        .filter(|&slot| describe(slot as u64).is_some_and(|desc| desc[0] == kind))
        .count()
}
fn cap_inventory_snapshot() -> CapInventory {
    let mut inventory = CapInventory {
        occupied: 0,
        digest: 0xcbf2_9ce4_8422_2325,
    };
    let saved = &raw mut REFUSAL_CAPS;
    for slot in 0..CAP_SLOTS {
        let desc = cap_slot_descriptor(slot);
        if desc[0] != 0 {
            inventory.occupied += 1;
        }
        unsafe { (*saved)[slot] = desc };
        inventory_digest_add(&mut inventory.digest, slot as u64);
        for word in desc {
            inventory_digest_add(&mut inventory.digest, word);
        }
    }
    inventory
}
fn cap_inventory_compare() -> (CapInventory, bool) {
    let mut inventory = CapInventory {
        occupied: 0,
        digest: 0xcbf2_9ce4_8422_2325,
    };
    let saved = &raw const REFUSAL_CAPS;
    let mut equal = true;
    for slot in 0..CAP_SLOTS {
        let desc = cap_slot_descriptor(slot);
        if desc[0] != 0 {
            inventory.occupied += 1;
        }
        if desc != unsafe { (*saved)[slot] } {
            equal = false;
        }
        inventory_digest_add(&mut inventory.digest, slot as u64);
        for word in desc {
            inventory_digest_add(&mut inventory.digest, word);
        }
    }
    (inventory, equal)
}
fn audit_full_session_caps() {
    let descriptors = &raw const REFUSAL_CAPS;
    let mut processes = 0usize;
    let mut regions = 0usize;
    let mut notifications = 0usize;
    for slot in 0..CAP_SLOTS {
        let desc = unsafe { (*descriptors)[slot] };
        match desc[0] {
            3 => notifications += 1,
            4 => {
                for prior in 0..slot {
                    let other = unsafe { (*descriptors)[prior] };
                    if other[0] == 4 && other[1] == desc[1] {
                        die(98)
                    }
                }
                processes += 1;
            }
            7 => {
                for prior in 0..slot {
                    let other = unsafe { (*descriptors)[prior] };
                    if other[0] == 7 && other[1] == desc[1] {
                        die(98)
                    }
                }
                regions += 1;
            }
            _ => {}
        }
    }
    if processes != LIMIT || regions != unsafe { SHARED_CAP_BASELINE } || notifications != LIMIT + 1
    {
        die(98)
    }
    log(b"[desktop] full-session cap audit distinct Process caps=");
    log_number(processes as u64);
    log(b" baseline SharedRegion caps=");
    log_number(regions as u64);
    log(b" distinct Notifications=");
    log_number(notifications as u64);
    log(b"\n");
}
fn log_capacity_refusal_inventory(before: CapInventory, resources_before: ResourceReceipt) {
    audit_full_session_caps();
    let (after, caps_equal) = cap_inventory_compare();
    let resources_after = observe_receipt();
    log(b"[desktop] full-session refusal cap inventory before occupied=");
    log_number(before.occupied);
    log(b" digest=");
    log_number(before.digest);
    log(b"\n[desktop] full-session refusal cap inventory after occupied=");
    log_number(after.occupied);
    log(b" digest=");
    log_number(after.digest);
    log(b"\n[desktop] full-session refusal cap inventory exact-equal=");
    if caps_equal {
        log(b"yes\n");
    } else {
        log(b"no\n");
    }
    log_resource_receipt(
        b"[desktop] full-session refusal resource inventory before=",
        resources_before,
    );
    log_resource_receipt(
        b"[desktop] full-session refusal resource inventory after=",
        resources_after,
    );
    log(b"[desktop] full-session refusal resource inventory exact-equal=");
    if caps_equal && resources_before == resources_after {
        log(b"yes\n");
    } else {
        log(b"no\n");
        die(98)
    }
}
fn destroy(slot: u64) {
    if slot != CAP_NONE {
        let rc = unsafe { syscall1(SYS_CAP_DESTROY, slot) };
        if rc != 0 {
            log(b"[desktop] cap destroy failed slot/status=");
            log_number(slot);
            log(b"/");
            log_number(rc as u64);
            log(b"\n");
            die(80)
        }
    }
}
fn child_capacity_available(instance: usize) -> bool {
    if instance >= LIMIT {
        return false;
    }
    unsafe { (&*(&raw const APP_GROUPS))[instance].as_ref() }.is_none_or(ProcessGroup::can_spawn)
}
fn child_live(instance: usize, handle: Handle) -> bool {
    unsafe { (&*(&raw const APP_GROUPS))[instance].as_ref() }
        .unwrap_or_else(|| die(83))
        .is_live(handle)
        .unwrap_or_else(|_| die(83))
}
fn child_exit_status(instance: usize, handle: Handle) -> u64 {
    unsafe { (&*(&raw const APP_GROUPS))[instance].as_ref() }
        .unwrap_or_else(|| die(83))
        .exit_status(handle)
        .unwrap_or_else(|_| die(83))
        .unwrap_or_else(|| die(83))
}
fn finish_spawn_result(
    outcome: Result<Handle, GroupSpawnError<SpawnError, ChildProcess>>,
) -> Result<Handle, i64> {
    match outcome {
        Ok(handle) => Ok(handle),
        Err(GroupSpawnError::Full) => Err(STATUS_BUSY),
        Err(GroupSpawnError::Spawn(SpawnError::Kernel(status))) => Err(status),
        // Every other spawn error is an invariant or capability-query failure
        // after the kernel may have created a child. Do not continue as if the
        // app manager owned it.
        Err(GroupSpawnError::Spawn(_)) => die(83),
        Err(GroupSpawnError::Record(InsertError::Full(mut child))) => {
            child
                .finish(FinishMode::StopAndReap)
                .unwrap_or_else(|_| die(83));
            Err(STATUS_BUSY)
        }
    }
}
fn finish_app_group(instance: usize) {
    let group =
        unsafe { (&mut *(&raw mut APP_GROUPS))[instance].as_mut() }.unwrap_or_else(|| die(84));
    let members = group.len();
    // An AppInstance owns all its processes. Retiring it always finishes every
    // exact Process cap in the group before releasing the group record.
    group.stop_all().unwrap_or_else(|_| die(84));
    if !group.is_empty() {
        die(84)
    }
    for member in unsafe { &mut SESSIONS[instance].helpers } {
        if member.active && member.timer_slot != u8::MAX {
            destroy(u64::from(member.timer_slot));
        }
        *member = EMPTY_HELPER;
    }
    log(b"[desktop] AppInstance ProcessGroup teardown members=");
    log_number(members as u64);
    log(b"; final members=0\n");
}

fn drain_session_output(index: usize, session: Session) {
    if session.stream_va == 0 || session.stream_faulted {
        return;
    }
    // SAFETY: the Desktop retains the exact mapping until AppInstance
    // retirement. A malformed guest ring is handled as a closed stream.
    let streams = match unsafe {
        arena_runtime::streams::StreamSet::attach(
            session.stream_va as *mut u8,
            arena_runtime::streams::STREAM_PAGE_BYTES,
        )
    } {
        Ok(streams) => streams,
        Err(_) => {
            log(b"[desktop] malformed native stream page; output disabled\n");
            unsafe { SESSIONS[index].stream_faulted = true };
            return;
        }
    };
    let mut changed = false;
    let mut buffer = [0u8; 256];
    for channel in [
        arena_runtime::streams::Channel::Stdout,
        arena_runtime::streams::Channel::Stderr,
    ] {
        let mut reader = match streams.reader(channel) {
            Ok(reader) => reader,
            Err(_) => {
                unsafe { SESSIONS[index].stream_faulted = true };
                return;
            }
        };
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => {
                    if reader.writer_closed() {
                        let (bit, name): (u8, &[u8]) = match channel {
                            arena_runtime::streams::Channel::Stdout => (1, b"stdout"),
                            arena_runtime::streams::Channel::Stderr => (2, b"stderr"),
                            arena_runtime::streams::Channel::Stdin => (0, b"stdin"),
                        };
                        if session.stream_eof & bit == 0 {
                            unsafe { SESSIONS[index].stream_eof |= bit };
                            log(b"[desktop] native stream channel ");
                            log(name);
                            log(b" reached EOF\n");
                        }
                    }
                    break;
                }
                Err(arena_runtime::streams::Error::WouldBlock) => break,
                Ok(count) => {
                    log(&buffer[..count]);
                    changed = true;
                }
                Err(_) => {
                    log(b"[desktop] native output ring refused; output disabled\n");
                    unsafe { SESSIONS[index].stream_faulted = true };
                    return;
                }
            }
        }
    }
    let mut stdin_changed = false;
    if session.stdin_count != 0
        && let Ok(mut writer) = streams.writer(arena_runtime::streams::Channel::Stdin)
    {
        let mut head = session.stdin_head;
        let mut count = session.stdin_count;
        while count != 0 {
            let byte = unsafe { SESSIONS[index].stdin_pending[usize::from(head)] };
            match writer.write(&[byte]) {
                Ok(1) => {
                    head = ((usize::from(head) + 1) % STREAM_INPUT_PENDING) as u8;
                    count -= 1;
                    stdin_changed = true;
                }
                Ok(_) | Err(arena_runtime::streams::Error::WouldBlock) => break,
                Err(_) => {
                    unsafe { SESSIONS[index].stream_faulted = true };
                    break;
                }
            }
        }
        unsafe {
            SESSIONS[index].stdin_head = head;
            SESSIONS[index].stdin_count = count;
        }
    }
    if changed || stdin_changed {
        // Freeing output capacity may unblock the exact AppInstance writer.
        let _ = unsafe { syscall2(SYS_NOTIFY, clock(index), BADGE_STREAM_READY) };
    }
}

fn queue_stdin_byte(index: usize, byte: u8) {
    let Some(session) = (unsafe { (&*(&raw const SESSIONS)).get(index).copied() }) else {
        return;
    };
    if session.stream_va == 0 || session.stream_faulted {
        return;
    }
    if session.stdin_count == 0 {
        let streams = match unsafe {
            arena_runtime::streams::StreamSet::attach(
                session.stream_va as *mut u8,
                arena_runtime::streams::STREAM_PAGE_BYTES,
            )
        } {
            Ok(streams) => streams,
            Err(_) => {
                unsafe { SESSIONS[index].stream_faulted = true };
                return;
            }
        };
        let mut writer = match streams.writer(arena_runtime::streams::Channel::Stdin) {
            Ok(writer) => writer,
            Err(_) => {
                unsafe { SESSIONS[index].stream_faulted = true };
                return;
            }
        };
        match writer.write(&[byte]) {
            Ok(1) => {
                let _ = unsafe { syscall2(SYS_NOTIFY, clock(index), BADGE_STREAM_READY) };
                return;
            }
            Err(arena_runtime::streams::Error::WouldBlock) => {}
            Err(
                arena_runtime::streams::Error::BrokenPipe | arena_runtime::streams::Error::Closed,
            ) => return,
            _ => {
                unsafe { SESSIONS[index].stream_faulted = true };
                return;
            }
        }
    }
    let count = usize::from(session.stdin_count);
    if count == STREAM_INPUT_PENDING {
        log(b"[desktop] native stdin queue full; keyboard byte refused\n");
        return;
    }
    let at = (usize::from(session.stdin_head) + count) % STREAM_INPUT_PENDING;
    unsafe {
        SESSIONS[index].stdin_pending[at] = byte;
        SESSIONS[index].stdin_count += 1;
    }
}

fn stream_owner_for_window(handle: u64) -> Option<usize> {
    if let Some(index) = unsafe {
        (&*(&raw const SESSIONS))
            .iter()
            .position(|session| session.id != 0 && session.handle == handle)
    } {
        return Some(index);
    }
    unsafe {
        (&*(&raw const EXTRA_WINDOWS))
            .iter()
            .find(|window| window.handle == handle)
            .map(|window| window.owner_session)
    }
}

fn stream_keyboard_byte(code: u16, _mods: u8) -> Option<u8> {
    // inputd has already decoded evdev events and applies shift state before
    // publishing this native input frame. Preserve the byte it delivered;
    // navigation and modifier records use values outside this byte set.
    let byte = u8::try_from(code).ok()?;
    matches!(byte, b'\x08' | b'\t' | b'\r' | b' '..=b'~').then_some(byte)
}

fn drain_standard_streams() {
    let sessions = unsafe { *(&raw const SESSIONS) };
    for (index, session) in sessions.into_iter().enumerate() {
        if session.id != 0 {
            drain_session_output(index, session);
        }
    }
}

fn retire_streams(session: Session) {
    if session.stream_va == 0 {
        return;
    }
    if !session.stream_faulted
        && let Ok(streams) = unsafe {
            arena_runtime::streams::StreamSet::attach(
                session.stream_va as *mut u8,
                arena_runtime::streams::STREAM_PAGE_BYTES,
            )
        }
    {
        if let Ok(mut stdin) = streams.writer(arena_runtime::streams::Channel::Stdin) {
            stdin.close();
        }
        for channel in [
            arena_runtime::streams::Channel::Stdout,
            arena_runtime::streams::Channel::Stderr,
        ] {
            if let Ok(mut reader) = streams.reader(channel) {
                reader.close();
            }
            if let Ok(mut writer) = streams.writer(channel) {
                writer.close();
            }
        }
    }
    // Unmap before dropping the owner cap so SharedRegion pins/refcounts
    // return together at exact AppInstance retirement.
    if unsafe { syscall6(SYS_SHARED_UNMAP, session.stream_va, 0, 0, 0, 0, 0) } != 0 {
        die(86)
    }
    destroy(session.stream_cap);
}

fn spawn_child(instance: usize, image: u64, grants: &[InheritGrant]) -> Result<Handle, i64> {
    if instance >= LIMIT {
        return Err(STATUS_BAD_ARG);
    }
    if image >= CAP_SLOTS as u64 {
        return Err(-2);
    }
    let spawn = || {
        ChildProcess::spawn(
            image as u8,
            grants,
            Some(ExitSignal {
                notification_slot: CLOCK as u8,
                badge: BADGE_EXIT,
            }),
        )
    };
    if let Some(group) = unsafe { (&mut *(&raw mut APP_GROUPS))[instance].as_mut() } {
        return finish_spawn_result(group.spawn(spawn));
    }
    let mut group = ProcessGroup::<ChildProcess, APP_GROUP_PROCESSES>::new();
    let handle = finish_spawn_result(group.spawn(spawn))?;
    unsafe { (*(&raw mut APP_GROUPS))[instance] = Some(group) };
    Ok(handle)
}

fn launch_helper_image_v2(
    instance: usize,
    image: arena_desktop::package::NativeHelper,
    helper_id: [u8; 32],
) -> Result<(Handle, u8), i64> {
    use arena_startup_abi::startup as startup_abi;

    let known_flags = arena_desktop::package::HELPER_FLAG_TIMER
        | arena_desktop::package::HELPER_FLAG_OWNER_SIGNAL;
    if instance >= LIMIT
        || image.flags & !known_flags != 0
        || (image.flags & arena_desktop::package::HELPER_FLAG_OWNER_SIGNAL != 0
            && image.flags & arena_desktop::package::HELPER_FLAG_TIMER == 0)
        || !child_capacity_available(instance)
    {
        destroy(image.image.capability);
        return Err(STATUS_BUSY);
    }
    let session = unsafe { SESSIONS[instance] };
    if session.id == 0 || session.kind != 6 || session.badge == 0 {
        destroy(image.image.capability);
        return Err(STATUS_BAD_ARG);
    }
    let ready = unsafe { syscall6(SYS_SPAWN_CHECK, image.image.capability, 0, 0, 0, 0, 0) };
    if ready != 0 {
        destroy(image.image.capability);
        return Err(ready);
    }
    let needed = if image.flags & arena_desktop::package::HELPER_FLAG_TIMER != 0 {
        3
    } else {
        2
    };
    let free = (0..CAP_SLOTS as u64)
        .filter(|slot| unsafe { syscall6(SYS_CAP_OCCUPIED, *slot, 0, 0, 0, 0, 0) } == 0)
        .count();
    if free < needed {
        destroy(image.image.capability);
        return Err(STATUS_BUSY);
    }
    let Some(id_len) = session
        .application_id
        .iter()
        .position(|byte| *byte == 0)
        .filter(|length| *length != 0 && *length < 32)
    else {
        destroy(image.image.capability);
        return Err(STATUS_BAD_ARG);
    };
    let Some(helper_len) = helper_id
        .iter()
        .position(|byte| *byte == 0)
        .filter(|length| *length != 0 && *length < 32)
    else {
        destroy(image.image.capability);
        return Err(STATUS_BAD_ARG);
    };
    let arguments: [&[u8]; 2] = [&session.application_id[..id_len], &helper_id[..helper_len]];
    let timer_granted = image.flags & arena_desktop::package::HELPER_FLAG_TIMER != 0;
    let owner_signal = image.flags & arena_desktop::package::HELPER_FLAG_OWNER_SIGNAL != 0;
    let descriptors = [
        startup_cap_descriptor(
            1,
            startup_abi::CAP_KIND_NOTIFICATION,
            RIGHTS_READ | RIGHTS_WRITE,
        ),
        startup_cap_descriptor(2, startup_abi::CAP_KIND_NOTIFICATION, RIGHTS_WRITE),
    ];
    let capability_count = usize::from(timer_granted) + usize::from(owner_signal);
    let capabilities = &descriptors[..capability_count];
    let spec = startup_abi::StartupSpec {
        application_id: &session.application_id,
        instance_slot: instance as u16,
        instance_generation: u64::from(session.badge),
        // Helper streams require an explicit AHL1 opt-in and endpoint grant;
        // the primary's standard-stream declaration is not ambient inheritance.
        flags: session.app_flags & !arena_startup_abi::manifest::FLAG_STANDARD_STREAMS,
        arguments: &arguments,
        environment: &[],
        capabilities,
        cwd: None,
        stdin: None,
        stdout: None,
        stderr: None,
        entry: image.image.entry,
        load_base: image.image.load_base,
        clock_us: arena_desktop::app_client::now(),
    };
    let mut startup_page = [0u8; startup_abi::BLOCK_BYTES];
    if startup_abi::encode(&spec, &mut startup_page).is_err() {
        destroy(image.image.capability);
        return Err(STATUS_BAD_ARG);
    }
    let mut startup_out = [0u64; 3];
    let status = unsafe {
        syscall6(
            SYS_SHARED_CREATE,
            POOL,
            1,
            startup_out.as_mut_ptr() as u64,
            0,
            0,
            0,
        )
    };
    if status != 0 {
        destroy(image.image.capability);
        return Err(status);
    }
    let startup_cap = startup_out[0];
    let startup_va = unsafe { syscall2(SYS_SHARED_MAP, startup_cap, 1) };
    if startup_va <= 0 {
        destroy(startup_cap);
        destroy(image.image.capability);
        return Err(startup_va);
    }
    unsafe {
        core::ptr::copy_nonoverlapping(
            startup_page.as_ptr(),
            startup_va as *mut u8,
            startup_abi::BLOCK_BYTES,
        );
    }
    if unsafe { syscall6(SYS_SHARED_UNMAP, startup_va as u64, 0, 0, 0, 0, 0) } != 0 {
        die(86)
    }
    let mut timer_slot = u8::MAX;
    if timer_granted {
        let Some(free_slot) = (45..125)
            .find(|slot| unsafe { syscall6(SYS_CAP_OCCUPIED, *slot as u64, 0, 0, 0, 0, 0) } == 0)
        else {
            destroy(startup_cap);
            destroy(image.image.capability);
            return Err(STATUS_BUSY);
        };
        let status = unsafe {
            syscall6(
                SYS_NOTIFICATION_CREATE,
                NOTIFICATION_FACTORY_SLOT,
                free_slot as u64,
                0,
                0,
                0,
                0,
            )
        };
        if status != 0 {
            destroy(startup_cap);
            destroy(image.image.capability);
            return Err(status);
        }
        timer_slot = free_slot as u8;
    }
    let mut grants = [
        InheritGrant::new(startup_cap as u8, (RIGHTS_READ | RIGHTS_DESTROY) as u32),
        InheritGrant::new(0, 0),
        InheritGrant::new(0, 0),
    ];
    let mut grant_count = 1;
    if timer_granted {
        grants[grant_count] = InheritGrant::new(timer_slot, (RIGHTS_READ | RIGHTS_WRITE) as u32);
        grant_count = 2;
    }
    if owner_signal {
        grants[grant_count] = InheritGrant::new(clock(instance) as u8, RIGHTS_WRITE as u32);
        grant_count += 1;
    }
    let process = spawn_child(instance, image.image.capability, &grants[..grant_count]);
    destroy(startup_cap);
    destroy(image.image.capability);
    match process {
        Ok(handle) => Ok((handle, timer_slot)),
        Err(status) => {
            if timer_slot != u8::MAX {
                destroy(u64::from(timer_slot));
            }
            Err(status)
        }
    }
}

fn spawn_installed_helper(instance: usize, helper_id: [u8; 32]) -> Result<u32, i64> {
    if instance >= LIMIT {
        return Err(STATUS_BAD_ARG);
    }
    let session = unsafe { SESSIONS[instance] };
    if session.id == 0 || session.kind != 6 || session.badge == 0 {
        return Err(STATUS_BAD_ARG);
    }
    let Some(record) = session.helpers.iter().position(|member| !member.active) else {
        return Err(STATUS_BUSY);
    };
    if !child_capacity_available(instance) {
        return Err(STATUS_BUSY);
    }
    let helper =
        arena_desktop::package::launch_installed_helper(&session.application_id, &helper_id, POOL)
            .map_err(|error| error as i64)?;
    let Some(helper_len) = helper_id.iter().position(|byte| *byte == 0) else {
        destroy(helper.image.capability);
        return Err(STATUS_BAD_ARG);
    };
    if helper_len == 0 || helper_len >= 32 {
        destroy(helper.image.capability);
        return Err(STATUS_BAD_ARG);
    }
    let (handle, timer_slot) = launch_helper_image_v2(instance, helper, helper_id)?;
    let raw = handle.as_raw();
    unsafe {
        SESSIONS[instance].helpers[record] = HelperMember {
            handle: raw,
            helper_id,
            timer_slot,
            active: true,
        };
    }
    log(b"[desktop] signed helper id=");
    log(&helper_id[..helper_len]);
    log(b" spawned in owning ProcessGroup handle=");
    log_number(u64::from(raw));
    log(b"\n");
    Ok(raw)
}

fn helper_record(instance: usize, raw: u32) -> Result<(usize, Handle), i64> {
    if instance >= LIMIT {
        return Err(STATUS_BAD_ARG);
    }
    let session = unsafe { *(&raw const SESSIONS).cast::<Session>().add(instance) };
    if session.id == 0 || raw == 0 {
        return Err(STATUS_BAD_ARG);
    }
    let Some(slot) = session
        .helpers
        .iter()
        .position(|member| member.active && member.handle == raw)
    else {
        return Err(STATUS_BAD_ARG);
    };
    Ok((slot, Handle::from_raw(raw)))
}

fn wait_helper(instance: usize, raw: u32, bytes: &mut [u8; 64]) -> Result<u64, i64> {
    use arena_desktop::service_wire::Frame as S;
    let (slot, handle) = helper_record(instance, raw)?;
    let group =
        unsafe { (&*(&raw const APP_GROUPS))[instance].as_ref() }.ok_or(STATUS_SERVICE_GONE)?;
    let Some(status) = group.exit_status(handle).map_err(|_| STATUS_BAD_ARG)? else {
        return Err(STATUS_BUSY);
    };
    unsafe { (&mut *(&raw mut APP_GROUPS))[instance].as_mut() }
        .ok_or(STATUS_SERVICE_GONE)?
        .reap_exited(handle)
        .map_err(|_| STATUS_BAD_ARG)?;
    let helper_id = unsafe { SESSIONS[instance].helpers[slot].helper_id };
    let timer_slot = unsafe { SESSIONS[instance].helpers[slot].timer_slot };
    if timer_slot != u8::MAX {
        destroy(u64::from(timer_slot));
    }
    unsafe { SESSIONS[instance].helpers[slot] = EMPTY_HELPER };
    *bytes = S::HelperExited {
        handle: raw,
        status,
    }
    .encode()
    .map_err(|_| STATUS_BAD_ARG)?;
    log(b"[desktop] helper id=");
    let helper_len = helper_id.iter().position(|byte| *byte == 0).unwrap_or(32);
    log(&helper_id[..helper_len]);
    log(b" Process-cap exit status=");
    log_number(status);
    log(b"; owner-group reap=ok\n");
    Ok(status)
}

fn terminate_helper(instance: usize, raw: u32, bytes: &mut [u8; 64]) -> Result<u64, i64> {
    use arena_desktop::service_wire::Frame as S;
    let (slot, handle) = helper_record(instance, raw)?;
    unsafe { (&mut *(&raw mut APP_GROUPS))[instance].as_mut() }
        .ok_or(STATUS_SERVICE_GONE)?
        .stop_and_reap(handle)
        .map_err(|_| STATUS_BAD_ARG)?;
    let helper_id = unsafe { SESSIONS[instance].helpers[slot].helper_id };
    let timer_slot = unsafe { SESSIONS[instance].helpers[slot].timer_slot };
    if timer_slot != u8::MAX {
        destroy(u64::from(timer_slot));
    }
    unsafe { SESSIONS[instance].helpers[slot] = EMPTY_HELPER };
    *bytes = S::HelperTerminated { handle: raw }
        .encode()
        .map_err(|_| STATUS_BAD_ARG)?;
    log(b"[desktop] helper id=");
    let helper_len = helper_id.iter().position(|byte| *byte == 0).unwrap_or(32);
    log(&helper_id[..helper_len]);
    log(b" Process-cap terminate/reap=ok\n");
    Ok(0)
}

fn helper_service(instance: usize, bytes: &mut [u8; 64]) -> Result<u64, i64> {
    use arena_desktop::service_wire::Frame as S;
    match S::decode(bytes).map_err(|_| STATUS_BAD_ARG)? {
        S::SpawnHelper { helper_id } => {
            let handle = spawn_installed_helper(instance, helper_id)?;
            *bytes = S::HelperStarted { handle }
                .encode()
                .map_err(|_| STATUS_BAD_ARG)?;
            Ok(u64::from(handle))
        }
        S::WaitHelper { handle } => wait_helper(instance, handle, bytes),
        S::TerminateHelper { handle } => terminate_helper(instance, handle, bytes),
        _ => Err(STATUS_BAD_ARG),
    }
}
fn display(frame: arena_compositor_model::wire::Frame, cap: u64) -> ([u64; 3], [u8; 64]) {
    let mut b = [0; 64];
    frame.encode(&mut b).unwrap_or_else(|_| die(81));
    let mut o = [0, 0, CAP_NONE];
    if unsafe {
        syscall6(
            SYS_IPC_CALL,
            DISPLAY,
            0,
            0,
            cap,
            o.as_mut_ptr() as u64,
            b.as_mut_ptr() as u64,
        )
    } != 0
    {
        die(82)
    }
    (o, b)
}
fn launch(kind: u8, path: [u8; 32], document: u64) -> Result<(), i64> {
    launch_with_document(kind, path, document, false)
}

fn launch_with_document(
    kind: u8,
    path: [u8; 32],
    document: u64,
    read_only: bool,
) -> Result<(), i64> {
    use arena_desktop::scope;
    // `path` is a title only (ADR-0077): printable, never authority.
    let n = path.iter().position(|b| *b == 0).unwrap_or(32);
    if kind > 5 || n == 32 || !path[..n].iter().all(|b| (0x20..0x7f).contains(b)) {
        return Err(-2);
    }
    // Files are reached through filesd capabilities, not name scopes: the
    // AFS1 `user-*` file functions are retired for applications.
    let (scope, function_rights) = match kind {
        0 | 1 => (
            scope::LAUNCH,
            RIGHTS_READ | RIGHTS_WRITE | RIGHTS_COPY | RIGHTS_DESTROY,
        ),
        2 => (0, RIGHTS_READ | RIGHTS_WRITE | RIGHTS_COPY | RIGHTS_DESTROY),
        3 => (
            scope::PREFERENCES,
            RIGHTS_WRITE | RIGHTS_COPY | RIGHTS_DESTROY,
        ),
        _ => (0, RIGHTS_READ | RIGHTS_COPY),
    };
    let launch_targets = match kind {
        0 => 0x3f,
        1 => 1 << 2,
        _ => 0,
    };
    launch_image_with_document(
        APPLICATION,
        kind,
        scope,
        function_rights,
        path,
        kind == 4,
        launch_targets,
        document,
        read_only,
    )
}

/// Launch a descriptive installed-catalog selection through packaged's
/// fresh receiver-side revalidation. The registry row and app ID never enter
/// SYS_SPAWN as authority; the exact returned Image capability does.
fn launch_installed_application(
    application_id: &[u8; 32],
    app_flags: u32,
    document: u64,
    read_only: bool,
) -> Result<(), i64> {
    use arena_desktop::package::manifest;
    if app_flags & manifest::FLAG_MULTI_INSTANCE == 0 {
        let existing = unsafe { &*(&raw const SESSIONS) }
            .iter()
            .find(|session| session.id != 0 && session.application_id == *application_id);
        if let Some(session) = existing {
            if document != CAP_NONE {
                return Err(STATUS_BUSY);
            }
            if session.handle != 0 {
                let _ = unsafe { (&mut *(&raw mut WM)).activate(session.handle) };
            }
            return Ok(());
        }
    }
    let image = arena_desktop::package::launch_installed(application_id, POOL)
        .map_err(|error| error as i64)?;
    if app_flags & manifest::FLAG_HEADLESS != 0 {
        if document != CAP_NONE {
            destroy(image.capability);
            return Err(STATUS_BAD_ARG);
        }
        let launched = launch_headless_image_v2(
            image.capability,
            *application_id,
            image.entry,
            image.load_base,
            app_flags,
        );
        destroy(image.capability);
        return launched;
    }
    let launched = launch_image_v2_for_app_with_document(
        image.capability,
        6,
        0,
        RIGHTS_READ | RIGHTS_COPY,
        [0; 32],
        false,
        0,
        document,
        *application_id,
        image.entry,
        image.load_base,
        app_flags,
        read_only,
    );
    destroy(image.capability);
    launched
}

/// Launch a signed headless application with no window service authority.
/// The only inherited application capability is its private clock
/// notification, attenuated to WAIT/TIMER rights. The generation-bearing
/// instance record and exact Process cap remain owned by the Desktop.
fn launch_headless_image_v2(
    image: u64,
    application_id: [u8; 32],
    entry: u64,
    load_base: u64,
    app_flags: u32,
) -> Result<(), i64> {
    use arena_startup_abi::manifest::FLAG_HEADLESS;
    use arena_startup_abi::startup as startup_abi;

    let known_flags = arena_startup_abi::manifest::FLAG_MULTI_INSTANCE
        | arena_startup_abi::manifest::FLAG_BACKGROUND
        | arena_startup_abi::manifest::FLAG_HEADLESS
        | FLAG_STANDARD_STREAMS;
    if app_flags & FLAG_HEADLESS == 0 || app_flags & !known_flags != 0 {
        return Err(STATUS_BAD_ARG);
    }
    let ready = unsafe { syscall6(SYS_SPAWN_CHECK, image, 0, 0, 0, 0, 0) };
    if ready != 0 {
        log_launch_refusal(b"headless-spawn-check", ready);
        return Err(ready);
    }
    let sessions = unsafe { &mut *(&raw mut SESSIONS) };
    let Some(i) = sessions.iter().position(|session| session.id == 0) else {
        log_launch_refusal(b"headless-instance-table", STATUS_BUSY);
        return Err(STATUS_BUSY);
    };
    if i >= startup_abi::INSTANCE_SLOTS || !child_capacity_available(i) {
        log_launch_refusal(b"headless-process-group", STATUS_BUSY);
        return Err(STATUS_BUSY);
    }
    let generation = unsafe { NEXT_SESSION_BADGE };
    if generation == 0 {
        log_launch_refusal(b"headless-instance-generation", STATUS_BUSY);
        return Err(STATUS_BUSY);
    }
    let free = (0..CAP_SLOTS as u64)
        .filter(|slot| describe(*slot).is_none())
        .count();
    let wants_streams = app_flags & FLAG_STANDARD_STREAMS != 0;
    if free < 3 + usize::from(wants_streams) {
        log_launch_refusal(b"headless-cap-slots", STATUS_BUSY);
        return Err(STATUS_BUSY);
    }
    let mut streams = if wants_streams {
        Some(PendingStream::create()?)
    } else {
        None
    };
    let id_len = application_id
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(32);
    if id_len == 0 || id_len == 32 {
        return Err(STATUS_BAD_ARG);
    }
    let arguments: [&[u8]; 1] = [&application_id[..id_len]];
    let mut descriptors = [
        startup_cap_descriptor(
            1,
            startup_abi::CAP_KIND_NOTIFICATION,
            RIGHTS_READ | RIGHTS_WRITE,
        ),
        startup_cap_descriptor(2, 0, 0),
        startup_cap_descriptor(3, 0, 0),
    ];
    let mut descriptor_count = 1;
    let mut stdin = None;
    let mut stdout = None;
    let mut stderr = None;
    if streams.is_some() {
        let stream_index = descriptor_count as u16;
        descriptors[descriptor_count] = startup_role_descriptor(
            (descriptor_count + 1) as u16,
            CapabilityRole::StandardStreamSet,
            startup_abi::CAP_KIND_SHARED_REGION,
            RIGHTS_READ | RIGHTS_WRITE,
        );
        descriptor_count += 1;
        descriptors[descriptor_count] = startup_role_descriptor(
            (descriptor_count + 1) as u16,
            CapabilityRole::StreamWake,
            startup_abi::CAP_KIND_NOTIFICATION,
            RIGHTS_WRITE,
        );
        descriptor_count += 1;
        stdin = Some(stream_index);
        stdout = Some(stream_index);
        stderr = Some(stream_index);
    }
    let spec = startup_abi::StartupSpec {
        application_id: &application_id,
        instance_slot: i as u16,
        instance_generation: u64::from(generation),
        flags: app_flags,
        arguments: &arguments,
        environment: &[],
        capabilities: &descriptors[..descriptor_count],
        cwd: None,
        stdin,
        stdout,
        stderr,
        entry,
        load_base,
        clock_us: arena_desktop::app_client::now(),
    };
    let mut startup_page = [0u8; startup_abi::BLOCK_BYTES];
    if startup_abi::encode(&spec, &mut startup_page).is_err() {
        return Err(STATUS_BAD_ARG);
    }

    let mut startup_out = [0; 3];
    let rc = unsafe {
        syscall6(
            SYS_SHARED_CREATE,
            POOL,
            1,
            startup_out.as_mut_ptr() as u64,
            0,
            0,
            0,
        )
    };
    if rc != 0 {
        log_launch_refusal(b"headless-startup-page", rc);
        return Err(rc);
    }
    let startup_cap = startup_out[0];
    let startup_va = unsafe { syscall2(SYS_SHARED_MAP, startup_cap, 1) };
    if startup_va <= 0 {
        destroy(startup_cap);
        log_launch_refusal(b"headless-startup-map", startup_va);
        return Err(startup_va);
    }
    unsafe {
        core::ptr::copy_nonoverlapping(
            startup_page.as_ptr(),
            startup_va as *mut u8,
            startup_abi::BLOCK_BYTES,
        );
    }
    let unmap = unsafe { syscall6(SYS_SHARED_UNMAP, startup_va as u64, 0, 0, 0, 0, 0) };
    if unmap != 0 {
        die(86)
    }

    let clock_rights = RIGHTS_READ | RIGHTS_WRITE;
    let mut grants = [
        InheritGrant::new(startup_cap as u8, (RIGHTS_READ | RIGHTS_DESTROY) as u32),
        InheritGrant::new(clock(i) as u8, clock_rights as u32),
        InheritGrant::new(0, 0),
        InheritGrant::new(0, 0),
    ];
    let mut grant_count = 2;
    if let Some(stream) = streams.as_ref() {
        grants[grant_count] =
            InheritGrant::new(stream.cap as u8, (RIGHTS_READ | RIGHTS_WRITE) as u32);
        grant_count += 1;
        grants[grant_count] = InheritGrant::new(CLOCK as u8, RIGHTS_WRITE as u32);
        grant_count += 1;
    }
    let process = spawn_child(i, image, &grants[..grant_count]);
    destroy(startup_cap);
    let process = match process {
        Ok(process) => process,
        Err(status) => {
            log_launch_refusal(b"headless-kernel-spawn", status);
            return Err(status);
        }
    };
    let (stream_cap, stream_va) = streams
        .take()
        .map(PendingStream::commit)
        .unwrap_or((CAP_NONE, 0));

    // The high bit separates this descriptive instance key from the small
    // SharedRegion object IDs used by window authentication. It grants no
    // authority and is never accepted in place of the held Process cap.
    unsafe { NEXT_SESSION_BADGE = generation.checked_add(1).unwrap_or(0) };
    let id = (1u64 << 63) | u64::from(generation);
    sessions[i] = Session {
        id,
        stream_cap,
        stream_va,
        process: Some(process),
        kind: 6,
        application_id,
        app_flags,
        badge: 0,
        ..EMPTY
    };
    transient_caps();
    log(b"[desktop] verified headless application spawned; ordinary windows=");
    log_number(unsafe { (&*(&raw const WM)).windows().count() as u64 });
    log(b"; no surface or Desktop endpoint inherited\n");
    Ok(())
}

/// The session's filesd lineage head and, for the terminal and Files, its
/// /Users/user capability (ADR-0077). Other kinds get file capabilities
/// only through the chooser.
fn file_grants(kind: u8, document: u64, read_only: bool) -> (u64, u64) {
    let Some(files) = (unsafe { &*(&raw const AFS2) }) else {
        return (CAP_NONE, CAP_NONE);
    };
    let Ok(head) = files.new_lineage(USER_ROOT) else {
        return (CAP_NONE, CAP_NONE);
    };
    // An Editor opened on a document: a capability for exactly that file,
    // no more than the offered one, in the new session's lineage.
    if matches!(kind, 2 | 6) && document != CAP_NONE {
        let rights = if read_only {
            arena_desktop::filesd_wire::R_READ
        } else {
            R_DOC
        };
        let opened = files.open_in(document, rights, head).or_else(|_| {
            if read_only {
                Err(arena_desktop::filesd_wire::S_DENIED)
            } else {
                files.open_in(document, arena_desktop::filesd_wire::R_READ, head)
            }
        });
        return match opened {
            Ok((doc, _)) => (head, doc),
            Err(_) => (head, CAP_NONE),
        };
    }
    if !matches!(kind, 0 | 1) {
        return (head, CAP_NONE);
    }
    match files.open_in(USER_ROOT, arena_desktop::filesd_wire::R_ALL, head) {
        Ok((home, _)) => (head, home),
        Err(_) => (head, CAP_NONE),
    }
}
fn revoke_files(head: u64) {
    if head == CAP_NONE {
        return;
    }
    // Retire the whole lineage in filesd, then drop the broker's slot.
    if let Some(files) = unsafe { &*(&raw const AFS2) } {
        let _ = files.revoke(USER_ROOT, head);
    }
    destroy(head);
}
#[allow(clippy::too_many_arguments)]
fn launch_image(
    image: u64,
    kind: u8,
    scope: u8,
    function_rights: u64,
    path: [u8; 32],
    diagnostics: bool,
    launch_targets: u8,
    document: u64,
) -> Result<(), i64> {
    launch_image_with_document(
        image,
        kind,
        scope,
        function_rights,
        path,
        diagnostics,
        launch_targets,
        document,
        false,
    )
}

#[allow(clippy::too_many_arguments)]
fn launch_image_with_document(
    image: u64,
    kind: u8,
    scope: u8,
    function_rights: u64,
    path: [u8; 32],
    diagnostics: bool,
    launch_targets: u8,
    document: u64,
    read_only: bool,
) -> Result<(), i64> {
    if kind < 6 {
        launch_image_v2_with_document(
            image,
            kind,
            scope,
            function_rights,
            path,
            diagnostics,
            launch_targets,
            document,
            read_only,
        )
    } else {
        launch_image_legacy_with_document(
            image,
            kind,
            scope,
            function_rights,
            path,
            diagnostics,
            launch_targets,
            document,
            read_only,
        )
    }
}
fn startup_cap_descriptor(slot: u16, kind: u8, rights: u64) -> CapabilityDescriptor {
    CapabilityDescriptor {
        slot,
        role: CapabilityRole::Other,
        kind,
        rights: rights as u32,
    }
}
fn startup_role_descriptor(
    slot: u16,
    role: CapabilityRole,
    kind: u8,
    rights: u64,
) -> CapabilityDescriptor {
    CapabilityDescriptor {
        slot,
        role,
        kind,
        rights: rights as u32,
    }
}
#[allow(clippy::too_many_arguments)]
fn launch_image_v2_with_document(
    image: u64,
    kind: u8,
    scope: u8,
    function_rights: u64,
    path: [u8; 32],
    diagnostics: bool,
    launch_targets: u8,
    document: u64,
    read_only: bool,
) -> Result<(), i64> {
    if kind >= 6 {
        return Err(-2);
    }
    launch_image_v2_for_app_with_document(
        image,
        kind,
        scope,
        function_rights,
        path,
        diagnostics,
        launch_targets,
        document,
        arena_desktop::apps::APPLICATION_IDS[kind as usize],
        arena_desktop::apps::APPLICATION_ENTRY,
        arena_desktop::apps::APPLICATION_LOAD_BASE,
        FLAG_MULTI_INSTANCE,
        read_only,
    )
}

#[allow(clippy::too_many_arguments)]
fn launch_image_v2_for_app_with_document(
    image: u64,
    kind: u8,
    scope: u8,
    function_rights: u64,
    path: [u8; 32],
    diagnostics: bool,
    launch_targets: u8,
    document: u64,
    application_id: [u8; 32],
    entry: u64,
    load_base: u64,
    app_flags: u32,
    document_read_only: bool,
) -> Result<(), i64> {
    use arena_startup_abi::startup as startup_abi;

    if kind > 6 {
        return Err(-2);
    }
    let ready = unsafe { syscall6(SYS_SPAWN_CHECK, image, 0, 0, 0, 0, 0) };
    if ready != 0 {
        log_launch_refusal(b"spawn-check", ready);
        return Err(ready);
    }
    let sessions = unsafe { &mut *(&raw mut SESSIONS) };
    let Some(i) = sessions.iter().position(|s| s.id == 0) else {
        log_launch_refusal(b"session-table", STATUS_BUSY);
        return Err(STATUS_BUSY);
    };
    if i >= arena_desktop::apps::STARTUP_INSTANCE_SLOTS {
        log_launch_refusal(b"startup-instance-slots", STATUS_BUSY);
        return Err(STATUS_BUSY);
    }
    if !child_capacity_available(i) {
        log_launch_refusal(b"process-group", STATUS_BUSY);
        return Err(STATUS_BUSY);
    }
    let badge = unsafe { NEXT_SESSION_BADGE };
    if badge == 0 {
        log_launch_refusal(b"session-badge-exhausted", STATUS_BUSY);
        return Err(STATUS_BUSY);
    }
    // The exact cap table must have room for the request's landed cap, the
    // session region/snapshot, filesd lineage/tail, Process, startup page and
    // minted badge. This is conservative for kinds without a tail.
    let free = (0..CAP_SLOTS as u64)
        .filter(|slot| describe(*slot).is_none())
        .count();
    let wants_streams = app_flags & FLAG_STANDARD_STREAMS != 0;
    if free < 8 + usize::from(wants_streams) {
        log_launch_refusal(b"cap-slots", STATUS_BUSY);
        return Err(STATUS_BUSY);
    }
    let path_len = path.iter().position(|byte| *byte == 0).unwrap_or(32);
    if path_len == 32
        || !path[..path_len]
            .iter()
            .all(|byte| (0x20..0x7f).contains(byte))
    {
        return Err(-2);
    }

    let reserve = unsafe { RESERVE };
    let mut out = [0; 3];
    let rc = unsafe {
        syscall6(
            SYS_SHARED_CREATE,
            POOL,
            reserve.shared_pages,
            out.as_mut_ptr() as u64,
            0,
            0,
            0,
        )
    };
    if rc != 0 {
        log_launch_refusal(b"shared-region", rc);
        return Err(rc);
    }
    let region = out[0];
    let id = out[1];
    let va = unsafe { syscall2(SYS_SHARED_MAP, region, 1) };
    if va <= 0 {
        log_launch_refusal(b"shared-map", va);
        destroy(region);
        return Err(va);
    }
    let mut snap = [0; 3];
    let rc = unsafe {
        syscall6(
            SYS_SHARED_CREATE,
            POOL,
            reserve.snapshot_pages,
            snap.as_mut_ptr() as u64,
            0,
            0,
            0,
        )
    };
    let snapshot = if rc == 0 {
        let mapped = unsafe { syscall2(SYS_SHARED_MAP, snap[0], 1) };
        if mapped <= 0 {
            log_launch_refusal(b"snapshot-map", mapped);
        }
        destroy(snap[0]);
        mapped
    } else {
        log_launch_refusal(b"snapshot-create", rc);
        rc
    };
    if snapshot <= 0 {
        log_launch_refusal(b"snapshot-region-or-map", snapshot);
        unsafe { syscall6(SYS_SHARED_UNMAP, va as u64, 0, 0, 0, 0, 0) };
        destroy(region);
        return Err(snapshot);
    }

    let mut streams = if wants_streams {
        match PendingStream::create() {
            Ok(streams) => Some(streams),
            Err(status) => {
                unsafe {
                    syscall6(SYS_SHARED_UNMAP, va as u64, 0, 0, 0, 0, 0);
                    syscall6(SYS_SHARED_UNMAP, snapshot as u64, 0, 0, 0, 0, 0);
                }
                destroy(region);
                log_launch_refusal(b"stream-region", status);
                return Err(status);
            }
        }
    } else {
        None
    };

    let (files_head, home) = file_grants(kind, document, document_read_only);
    unsafe { syscall1(SYS_TRY_WAIT, clock(i)) };
    let clock_rights = if kind == 1 {
        RIGHTS_READ | RIGHTS_WRITE | RIGHTS_COPY
    } else {
        RIGHTS_READ | RIGHTS_WRITE
    };
    let tail = if diagnostics {
        InheritGrant::new(POOL as u8, RIGHTS_READ as u32)
    } else if home != CAP_NONE {
        InheritGrant::new(home as u8, (RIGHTS_WRITE | RIGHTS_COPY) as u32)
    } else {
        InheritGrant::new(0, 0)
    };
    let has_tail = diagnostics || home != CAP_NONE;

    // Only the READ-side holder can mint a badge. The kernel stores it in the
    // endpoint capability, hides it from SYS_CAP_DESCRIBE, and delivers it as
    // trusted receive metadata; it is never supplied in the request bytes.
    let badge_slot = unsafe {
        syscall6(
            SYS_ENDPOINT_MINT,
            SERVER,
            u64::from(badge),
            V2_BADGE_RIGHTS,
            0,
            0,
            0,
        )
    };
    if badge_slot < 0 {
        log_launch_refusal(b"session-badge", badge_slot);
        destroy(home);
        revoke_files(files_head);
        unsafe {
            syscall6(SYS_SHARED_UNMAP, va as u64, 0, 0, 0, 0, 0);
            syscall6(SYS_SHARED_UNMAP, snapshot as u64, 0, 0, 0, 0, 0);
        }
        destroy(region);
        return Err(badge_slot);
    }
    // Never reuse a badge, even when a later allocation/spawn step refuses.
    unsafe { NEXT_SESSION_BADGE = badge.checked_add(1).unwrap_or(0) };

    let id_len = application_id
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(32);
    if id_len == 32 {
        return Err(-2);
    }
    let mut arguments: [&[u8]; 2] = [&application_id[..id_len], &[]];
    let argument_count = if path_len == 0 {
        1
    } else {
        arguments[1] = &path[..path_len];
        2
    };
    let theme_env: &[u8] = if unsafe { PREFS.dark } {
        b"ARENA_THEME=dark"
    } else {
        b"ARENA_THEME=light"
    };
    let motion_env: &[u8] = if unsafe { PREFS.motion } {
        b"ARENA_MOTION=1"
    } else {
        b"ARENA_MOTION=0"
    };
    let environment = [theme_env, motion_env];
    let mut descriptors = [
        startup_cap_descriptor(
            V2_ENDPOINT_SLOT as u16,
            startup_abi::CAP_KIND_BADGED_ENDPOINT,
            V2_BADGE_RIGHTS,
        ),
        startup_cap_descriptor(
            V2_SURFACE_SLOT as u16,
            startup_abi::CAP_KIND_SHARED_REGION,
            RIGHTS_READ | RIGHTS_WRITE | RIGHTS_COPY,
        ),
        startup_cap_descriptor(
            V2_CLOCK_SLOT as u16,
            startup_abi::CAP_KIND_NOTIFICATION,
            clock_rights,
        ),
        startup_cap_descriptor(4, 0, 0),
        startup_cap_descriptor(5, 0, 0),
        startup_cap_descriptor(6, 0, 0),
    ];
    let mut descriptor_count = 3;
    if has_tail {
        descriptors[descriptor_count] = startup_cap_descriptor(
            (descriptor_count + 1) as u16,
            if diagnostics {
                startup_abi::CAP_KIND_MEMORY_POOL
            } else {
                startup_abi::CAP_KIND_BADGED_ENDPOINT
            },
            if diagnostics {
                RIGHTS_READ
            } else {
                RIGHTS_WRITE | RIGHTS_COPY
            },
        );
        descriptor_count += 1;
    }
    let mut stdin = None;
    let mut stdout = None;
    let mut stderr = None;
    if streams.is_some() {
        let stream_index = descriptor_count as u16;
        descriptors[descriptor_count] = startup_role_descriptor(
            (descriptor_count + 1) as u16,
            CapabilityRole::StandardStreamSet,
            startup_abi::CAP_KIND_SHARED_REGION,
            RIGHTS_READ | RIGHTS_WRITE,
        );
        descriptor_count += 1;
        descriptors[descriptor_count] = startup_role_descriptor(
            (descriptor_count + 1) as u16,
            CapabilityRole::StreamWake,
            startup_abi::CAP_KIND_NOTIFICATION,
            RIGHTS_WRITE,
        );
        descriptor_count += 1;
        stdin = Some(stream_index);
        stdout = Some(stream_index);
        stderr = Some(stream_index);
    }
    let spec = startup_abi::StartupSpec {
        application_id: &application_id,
        // The app-instance slot is the actual reserved 32-entry manager
        // session slot; it is never folded onto a live entry. The monotonic
        // generation distinguishes later reuse. Neither field authorizes a
        // syscall or selects a Desktop session.
        instance_slot: i as u16,
        instance_generation: u64::from(badge),
        flags: app_flags,
        arguments: &arguments[..argument_count],
        environment: &environment,
        capabilities: &descriptors[..descriptor_count],
        cwd: None,
        stdin,
        stdout,
        stderr,
        entry,
        load_base,
        clock_us: arena_desktop::app_client::now(),
    };
    let mut startup_page = [0u8; startup_abi::BLOCK_BYTES];
    if startup_abi::encode(&spec, &mut startup_page).is_err() {
        destroy(badge_slot as u64);
        destroy(home);
        revoke_files(files_head);
        unsafe {
            syscall6(SYS_SHARED_UNMAP, va as u64, 0, 0, 0, 0, 0);
            syscall6(SYS_SHARED_UNMAP, snapshot as u64, 0, 0, 0, 0, 0);
        }
        destroy(region);
        return Err(-2);
    }

    let mut startup_out = [0; 3];
    let rc = unsafe {
        syscall6(
            SYS_SHARED_CREATE,
            POOL,
            1,
            startup_out.as_mut_ptr() as u64,
            0,
            0,
            0,
        )
    };
    if rc != 0 {
        log_launch_refusal(b"startup-page", rc);
        destroy(badge_slot as u64);
        destroy(home);
        revoke_files(files_head);
        unsafe {
            syscall6(SYS_SHARED_UNMAP, va as u64, 0, 0, 0, 0, 0);
            syscall6(SYS_SHARED_UNMAP, snapshot as u64, 0, 0, 0, 0, 0);
        }
        destroy(region);
        return Err(rc);
    }
    let startup_cap = startup_out[0];
    // Reuse the already-reserved session mapping VA for this private staging
    // page. Mapping a third, one-page slot for every launch would permanently
    // warm an otherwise-unused page-table frame in the long-lived broker.
    if unsafe { syscall6(SYS_SHARED_UNMAP, va as u64, 0, 0, 0, 0, 0) } != 0 {
        die(86)
    }
    let startup_va = unsafe { syscall2(SYS_SHARED_MAP, startup_cap, 1) };
    if startup_va <= 0 || startup_va as u64 != va as u64 {
        let status = if startup_va <= 0 {
            startup_va
        } else {
            STATUS_BUSY
        };
        log_launch_refusal(b"startup-map", status);
        if startup_va > 0 {
            unsafe { syscall6(SYS_SHARED_UNMAP, startup_va as u64, 0, 0, 0, 0, 0) };
        }
        destroy(startup_cap);
        destroy(badge_slot as u64);
        destroy(home);
        revoke_files(files_head);
        unsafe {
            syscall6(SYS_SHARED_UNMAP, va as u64, 0, 0, 0, 0, 0);
            syscall6(SYS_SHARED_UNMAP, snapshot as u64, 0, 0, 0, 0, 0);
        }
        destroy(region);
        return Err(status);
    }
    unsafe {
        core::ptr::copy_nonoverlapping(
            startup_page.as_ptr(),
            startup_va as *mut u8,
            startup_abi::BLOCK_BYTES,
        );
    }
    if unsafe { syscall6(SYS_SHARED_UNMAP, startup_va as u64, 0, 0, 0, 0, 0) } != 0 {
        die(86)
    }
    let restored_va = unsafe { syscall2(SYS_SHARED_MAP, region, 1) };
    if restored_va <= 0 || restored_va as u64 != va as u64 {
        let status = if restored_va <= 0 {
            restored_va
        } else {
            STATUS_BUSY
        };
        log_launch_refusal(b"surface-remap", status);
        if restored_va > 0 {
            unsafe { syscall6(SYS_SHARED_UNMAP, restored_va as u64, 0, 0, 0, 0, 0) };
        }
        destroy(startup_cap);
        destroy(badge_slot as u64);
        destroy(home);
        revoke_files(files_head);
        unsafe {
            syscall6(SYS_SHARED_UNMAP, va as u64, 0, 0, 0, 0, 0);
            syscall6(SYS_SHARED_UNMAP, snapshot as u64, 0, 0, 0, 0, 0);
        }
        destroy(region);
        return Err(status);
    }

    let mut grants = [
        InheritGrant::new(startup_cap as u8, (RIGHTS_READ | RIGHTS_DESTROY) as u32),
        InheritGrant::new(badge_slot as u8, V2_BADGE_RIGHTS as u32),
        InheritGrant::new(
            region as u8,
            (RIGHTS_READ | RIGHTS_WRITE | RIGHTS_COPY) as u32,
        ),
        InheritGrant::new(clock(i) as u8, clock_rights as u32),
        InheritGrant::new(0, 0),
        InheritGrant::new(0, 0),
        InheritGrant::new(0, 0),
    ];
    let mut grant_count = 4;
    if has_tail {
        grants[grant_count] = tail;
        grant_count += 1;
    }
    if let Some(stream) = streams.as_ref() {
        grants[grant_count] =
            InheritGrant::new(stream.cap as u8, (RIGHTS_READ | RIGHTS_WRITE) as u32);
        grant_count += 1;
        grants[grant_count] = InheritGrant::new(CLOCK as u8, RIGHTS_WRITE as u32);
        grant_count += 1;
    }
    let process = spawn_child(i, image, &grants[..grant_count]);
    // Parent-side seed references are transient; the exact child caps remain.
    destroy(home);
    destroy(startup_cap);
    destroy(badge_slot as u64);
    let process = match process {
        Ok(process) => process,
        Err(status) => {
            log_launch_refusal(b"kernel-spawn", status);
            revoke_files(files_head);
            unsafe {
                syscall6(SYS_SHARED_UNMAP, va as u64, 0, 0, 0, 0, 0);
                syscall6(SYS_SHARED_UNMAP, snapshot as u64, 0, 0, 0, 0, 0);
            }
            destroy(region);
            return Err(status);
        }
    };
    let (stream_cap, stream_va) = streams
        .take()
        .map(PendingStream::commit)
        .unwrap_or((CAP_NONE, 0));
    destroy(region);
    sessions[i] = Session {
        id,
        va: va as u64,
        stream_cap,
        stream_va,
        process: Some(process),
        kind,
        application_id,
        app_flags,
        scope,
        function_rights,
        badge,
        launch_targets,
        path,
        snapshot: snapshot as u64,
        files_head,
        ..EMPTY
    };
    transient_caps();
    log(b"[desktop] real application spawned; ABI-v2 held Process, unique badge and surface bound\n");
    Ok(())
}
#[allow(clippy::too_many_arguments)]
fn launch_image_legacy_with_document(
    image: u64,
    kind: u8,
    scope: u8,
    function_rights: u64,
    path: [u8; 32],
    diagnostics: bool,
    launch_targets: u8,
    document: u64,
    document_read_only: bool,
) -> Result<(), i64> {
    let ready = unsafe { syscall6(SYS_SPAWN_CHECK, image, 0, 0, 0, 0, 0) };
    if ready != 0 {
        log_launch_refusal(b"spawn-check", ready);
        return Err(ready);
    }
    // The session table reserves original lifecycle owners, including children
    // that have not yet requested their window. No numerical caller identity.
    let sessions = unsafe { &mut *(&raw mut SESSIONS) };
    let Some(i) = sessions.iter().position(|s| s.id == 0) else {
        log_launch_refusal(b"session-table", STATUS_BUSY);
        return Err(STATUS_BUSY);
    };
    if !child_capacity_available(i) {
        log_launch_refusal(b"process-group", STATUS_BUSY);
        return Err(STATUS_BUSY);
    }
    // Region, snapshot (transiently), Process, the filesd lineage head,
    // the home grant and a lent copy (transiently). The request asking
    // for this launch has already landed its own cap, which is counted
    // as used here (it was once reserved twice; ADR-0079's Desktop watch
    // record took that slack).
    let free = (0..CAP_SLOTS as u64)
        .filter(|s| describe(*s).is_none())
        .count();
    if free < 6 {
        log_launch_refusal(b"cap-slots", STATUS_BUSY);
        return Err(STATUS_BUSY);
    }
    let reserve = unsafe { RESERVE };
    let mut out = [0; 3];
    let rc = unsafe {
        syscall6(
            SYS_SHARED_CREATE,
            POOL,
            reserve.shared_pages,
            out.as_mut_ptr() as u64,
            0,
            0,
            0,
        )
    };
    if rc != 0 {
        log_launch_refusal(b"shared-region", rc);
        return Err(rc);
    }
    let region = out[0];
    let id = out[1];
    let va = unsafe { syscall2(SYS_SHARED_MAP, region, 1) };
    if va <= 0 {
        log_launch_refusal(b"shared-map", va);
        destroy(region);
        return Err(va);
    }
    // The private snapshot lives as long as the broker's mapping of it: the
    // capability is dropped at once, so no other process can ever be given
    // it and it costs no capability slot.
    let mut snap = [0; 3];
    let rc = unsafe {
        syscall6(
            SYS_SHARED_CREATE,
            POOL,
            reserve.snapshot_pages,
            snap.as_mut_ptr() as u64,
            0,
            0,
            0,
        )
    };
    let snapshot = if rc == 0 {
        let mapped = unsafe { syscall2(SYS_SHARED_MAP, snap[0], 1) };
        if mapped <= 0 {
            log_launch_refusal(b"snapshot-map", mapped);
        }
        destroy(snap[0]);
        mapped
    } else {
        log_launch_refusal(b"snapshot-create", rc);
        rc
    };
    if snapshot <= 0 {
        log_launch_refusal(b"snapshot-region-or-map", snapshot);
        unsafe {
            syscall6(SYS_SHARED_UNMAP, va as u64, 0, 0, 0, 0, 0);
        }
        destroy(region);
        return Err(snapshot);
    }
    // Distinct inherited function reference to the exact fresh region. Its
    // marker rights do not enlarge the session's provisioned function scope.
    // Child slot 4: diagnostics (Monitor) or the /Users/user grant.
    let (files_head, home) = file_grants(kind, document, document_read_only);
    // A previous session's pending bits never reach this one.
    unsafe {
        syscall1(SYS_TRY_WAIT, clock(i));
    }
    // Files lends its clock to filesd for its folder watch (ADR-0079).
    let clock_rights = if kind == 1 {
        RIGHTS_READ | RIGHTS_WRITE | RIGHTS_COPY
    } else {
        RIGHTS_READ | RIGHTS_WRITE
    };
    let tail = if diagnostics {
        InheritGrant::new(POOL as u8, RIGHTS_READ as u32)
    } else if home != CAP_NONE {
        InheritGrant::new(home as u8, (RIGHTS_WRITE | RIGHTS_COPY) as u32)
    } else {
        InheritGrant::new(0, 0)
    };
    let spec = [
        InheritGrant::new(CALL_SIDE as u8, RIGHTS_WRITE as u32),
        InheritGrant::new(
            region as u8,
            (RIGHTS_READ | RIGHTS_WRITE | RIGHTS_COPY) as u32,
        ),
        InheritGrant::new(region as u8, function_rights as u32),
        InheritGrant::new(clock(i) as u8, clock_rights as u32),
        tail,
    ];
    let grant_count = if diagnostics || home != CAP_NONE {
        5
    } else {
        4
    };
    let process = spawn_child(i, image, &spec[..grant_count]);
    // The child holds its own copy; the broker never keeps the grant.
    destroy(home);
    let process = match process {
        Ok(process) => process,
        Err(status) => {
            log_launch_refusal(b"kernel-spawn", status);
            revoke_files(files_head);
            unsafe {
                syscall6(SYS_SHARED_UNMAP, va as u64, 0, 0, 0, 0, 0);
                syscall6(SYS_SHARED_UNMAP, snapshot as u64, 0, 0, 0, 0, 0);
            }
            destroy(region);
            return Err(status);
        }
    };
    // The exact child SharedRegion cap now owns the delegated authority.
    // This process keeps its two mapping pins, which are sufficient for
    // rendering and exact-va unmap after the held Process is reaped.
    destroy(region);
    sessions[i] = Session {
        id,
        va: va as u64,
        process: Some(process),
        kind,
        application_id: if kind < 6 {
            arena_desktop::apps::APPLICATION_IDS[kind as usize]
        } else {
            [0; 32]
        },
        scope,
        function_rights,
        badge: 0,
        launch_targets,
        path,
        snapshot: snapshot as u64,
        files_head,
        ..EMPTY
    };
    transient_caps();
    log(b"[desktop] real application spawned; held Process and own region bound\n");
    Ok(())
}
/// The dock (or F-key) for `kind` first brings back a minimized window of
/// that kind; only when there is none does it launch a new instance.
fn restore_minimized(kind: u8) -> bool {
    let sessions = unsafe { &*(&raw const SESSIONS) };
    let state = unsafe { &mut *(&raw mut WM) };
    let pick = state.minimized_window(|backing| {
        sessions
            .iter()
            .any(|s| s.id == backing && s.kind == kind && s.handle != 0)
    });
    pick.is_some_and(|h| state.activate(h).is_ok())
}
fn free_cap_slot() -> Option<u64> {
    (0..CAP_SLOTS as u64).find(|slot| describe(*slot).is_none())
}
/// Allocate one independently backed ordinary window for an already
/// authenticated application session. The broker keeps only its mapping;
/// the exact read/write/copy cap is transferred in the checked IPC reply.
fn create_extra_window(
    state: &mut State,
    owner_index: usize,
    owner_id: u64,
    width: u16,
    height: u16,
) -> Result<u64, i64> {
    let extra_index = unsafe { &*(&raw const EXTRA_WINDOWS) }
        .iter()
        .position(|window| window.handle == 0)
        .ok_or(STATUS_BUSY)?;
    let reserve = unsafe { RESERVE };
    if u64::from(width) * u64::from(height) > reserve.surface_pages * 1024 {
        return Err(-2);
    }
    let mut region_out = [0; 3];
    let rc = unsafe {
        syscall6(
            SYS_SHARED_CREATE,
            POOL,
            reserve.shared_pages,
            region_out.as_mut_ptr() as u64,
            0,
            0,
            0,
        )
    };
    if rc != 0 {
        return Err(rc);
    }
    let region = region_out[0];
    let backing_id = region_out[1];
    let va = unsafe { syscall2(SYS_SHARED_MAP, region, 1) };
    if va <= 0 {
        destroy(region);
        return Err(va);
    }
    let mut snapshot_out = [0; 3];
    let rc = unsafe {
        syscall6(
            SYS_SHARED_CREATE,
            POOL,
            reserve.snapshot_pages,
            snapshot_out.as_mut_ptr() as u64,
            0,
            0,
            0,
        )
    };
    if rc != 0 {
        unsafe { syscall6(SYS_SHARED_UNMAP, va as u64, 0, 0, 0, 0, 0) };
        destroy(region);
        return Err(rc);
    }
    let snapshot = unsafe { syscall2(SYS_SHARED_MAP, snapshot_out[0], 1) };
    destroy(snapshot_out[0]);
    if snapshot <= 0 {
        unsafe { syscall6(SYS_SHARED_UNMAP, va as u64, 0, 0, 0, 0, 0) };
        destroy(region);
        return Err(snapshot);
    }
    let Some(transfer_slot) = free_cap_slot() else {
        unsafe {
            syscall6(SYS_SHARED_UNMAP, va as u64, 0, 0, 0, 0, 0);
            syscall6(SYS_SHARED_UNMAP, snapshot as u64, 0, 0, 0, 0, 0);
        }
        destroy(region);
        return Err(STATUS_BUSY);
    };
    // DESTROY is scoped to dropping this newly created window cap. It is
    // needed both for the broker's checked-reply copy cleanup and for the
    // owner to release the independently held surface explicitly.
    let rights = RIGHTS_READ | RIGHTS_WRITE | RIGHTS_COPY | RIGHTS_DESTROY;
    let rc = unsafe { syscall3(SYS_CAP_COPY, region, transfer_slot, rights) };
    destroy(region);
    if rc != 0 {
        unsafe {
            syscall6(SYS_SHARED_UNMAP, va as u64, 0, 0, 0, 0, 0);
            syscall6(SYS_SHARED_UNMAP, snapshot as u64, 0, 0, 0, 0, 0);
        }
        return Err(rc);
    }
    let handle = match state.create_owned(owner_id, backing_id, width, height) {
        Ok(handle) => handle,
        Err(_) => {
            destroy(transfer_slot);
            unsafe {
                syscall6(SYS_SHARED_UNMAP, va as u64, 0, 0, 0, 0, 0);
                syscall6(SYS_SHARED_UNMAP, snapshot as u64, 0, 0, 0, 0, 0);
            }
            return Err(-2);
        }
    };
    let now = arena_desktop::app_client::now();
    let mut window = EMPTY_EXTRA;
    window.owner_session = owner_index;
    window.handle = handle;
    window.backing_id = backing_id;
    window.va = va as u64;
    window.snapshot = snapshot as u64;
    window.surface = (width, height);
    window.reveal.retarget(
        i32::from(height),
        now,
        if unsafe { PREFS.motion } {
            arena_ui::motion::OPEN_US
        } else {
            0
        },
        arena_ui::motion::Easing::Smooth,
    );
    unsafe {
        EXTRA_WINDOWS[extra_index] = window;
        REPLY_CAP = transfer_slot;
    }
    Ok(handle)
}
fn retire_extra(extra_index: usize) {
    let window = unsafe { EXTRA_WINDOWS[extra_index] };
    if window.handle == 0 {
        return;
    }
    unsafe { (&mut *(&raw mut WM)).retire(window.handle) }.unwrap_or_else(|_| die(85));
    if unsafe { syscall6(SYS_SHARED_UNMAP, window.va, 0, 0, 0, 0, 0) } != 0
        || unsafe { syscall6(SYS_SHARED_UNMAP, window.snapshot, 0, 0, 0, 0, 0) } != 0
    {
        die(86)
    }
    unsafe { EXTRA_WINDOWS[extra_index] = EMPTY_EXTRA };
}
fn retire_primary_window(index: usize) {
    let session = unsafe { SESSIONS[index] };
    if session.handle == 0 {
        return;
    }
    unsafe { (&mut *(&raw mut WM)).retire(session.handle) }.unwrap_or_else(|_| die(85));
    unsafe {
        let current = &mut *(&raw mut SESSIONS).cast::<Session>().add(index);
        current.handle = 0;
        current.title = [0; 32];
        current.published = false;
        current.close_pending = false;
        current.ending = false;
        current.content = 0;
        current.regions = compose::Regions::NONE;
        current.surface = (0, 0);
        current.popup = NO_POPUP;
        current.reveal = arena_ui::motion::Motion::fixed(arena_ui::metrics::TITLE_HEIGHT);
        current.focus = arena_ui::motion::Motion::fixed(0);
        current.reveal_last = arena_ui::metrics::TITLE_HEIGHT;
        current.focus_last = 0;
    }
}
fn retire(index: usize, _force: bool) {
    let s = unsafe { SESSIONS[index] };
    if s.id == 0 {
        return;
    }
    drain_session_output(index, s);
    finish_app_group(index);
    retire_streams(s);
    let mut extra_index = 0;
    while extra_index < EXTRA_LIMIT {
        let extra = unsafe { EXTRA_WINDOWS[extra_index] };
        if extra.handle != 0 && extra.owner_session == index {
            retire_extra(extra_index);
        }
        extra_index += 1;
    }
    if s.handle != 0 {
        unsafe { (&mut *(&raw mut WM)).retire(s.handle) }.unwrap_or_else(|_| die(85));
    }
    if s.va != 0 && unsafe { syscall6(SYS_SHARED_UNMAP, s.va, 0, 0, 0, 0, 0) } != 0 {
        die(86)
    }
    if s.snapshot != 0 && unsafe { syscall6(SYS_SHARED_UNMAP, s.snapshot, 0, 0, 0, 0, 0) } != 0 {
        die(86)
    }
    if unsafe { (*(&raw const CHOOSER)).as_ref() }.is_some_and(|c| c.session == index) {
        chooser_close();
    }
    if unsafe { OPEN_WITH_OWNER == index && OPEN_WITH_DOCUMENT != CAP_NONE } {
        close_all_applications();
    }
    destroy(s.grant);
    if s.offered_file != CAP_NONE {
        if let Some(files) = unsafe { &*(&raw const AFS2) } {
            files.release(s.offered_file);
        } else {
            destroy(s.offered_file);
        }
    }
    revoke_files(s.files_head);
    let _ = unsafe { syscall1(SYS_TRY_WAIT, clock(index)) };
    unsafe {
        SESSIONS[index] = EMPTY;
        APP_GROUPS[index] = None;
        WOKEN[index] = false;
    }
    log(b"[desktop] application retired: kind=");
    log_number(s.kind as u64);
    log(b" Process consumed; mapping and region released\n");
}
fn sweep() -> bool {
    let mut changed = false;
    let now = arena_desktop::app_client::now();
    for (i, s) in unsafe { *(&raw const SESSIONS) }.into_iter().enumerate() {
        if s.id != 0 && !child_live(i, s.process.unwrap_or_else(|| die(83))) {
            if !s.ending {
                let status = child_exit_status(i, s.process.unwrap_or_else(|| die(83)));
                log(b"[desktop] child Process-cap exit status=");
                log_number(status);
                log(b"\n");
            }
            if !s.ending && s.handle != 0 {
                unsafe {
                    SESSIONS[i].ending = true;
                    SESSIONS[i].reveal.retarget(
                        0,
                        now,
                        if PREFS.motion {
                            arena_ui::motion::CLOSE_US
                        } else {
                            0
                        },
                        arena_ui::motion::Easing::Smooth,
                    );
                }
                changed = true;
            } else if !s.reveal.active(now) {
                retire(i, false);
                changed = true;
            }
        }
    }
    changed
}
fn animate() -> bool {
    let now = arena_desktop::app_client::now();
    let state = unsafe { &*(&raw const WM) };
    let mut changed = false;
    if now / 1_000_000 != unsafe { UPTIME_SECOND } {
        unsafe { UPTIME_SECOND = now / 1_000_000 };
        changed = true;
    }
    if unsafe { NOTICE }.is_some_and(|(_, until)| until <= now) {
        unsafe { NOTICE = None };
        changed = true;
    }
    for s in unsafe { &mut *(&raw mut SESSIONS) }
        .iter_mut()
        .filter(|s| s.handle != 0)
    {
        let target = if state.focused() == Some(s.handle) {
            65536
        } else {
            0
        };
        if s.focus.target() != target {
            s.focus.retarget(
                target,
                now,
                if unsafe { PREFS.motion } {
                    arena_ui::motion::FOCUS_US
                } else {
                    0
                },
                arena_ui::motion::Easing::Smooth,
            );
            changed = true;
        }
        if !unsafe { PREFS.motion } {
            s.reveal
                .retarget(s.reveal.target(), now, 0, arena_ui::motion::Easing::Linear);
            s.focus
                .retarget(target, now, 0, arena_ui::motion::Easing::Linear);
        }
        let reveal = s.reveal.sample(now);
        let focus = s.focus.sample(now);
        changed |= reveal != s.reveal_last || focus != s.focus_last;
        s.reveal_last = reveal;
        s.focus_last = focus;
    }
    for w in unsafe { &mut *(&raw mut EXTRA_WINDOWS) }
        .iter_mut()
        .filter(|window| window.handle != 0)
    {
        let target = if state.focused() == Some(w.handle) {
            65536
        } else {
            0
        };
        if w.focus.target() != target {
            w.focus.retarget(
                target,
                now,
                if unsafe { PREFS.motion } {
                    arena_ui::motion::FOCUS_US
                } else {
                    0
                },
                arena_ui::motion::Easing::Smooth,
            );
            changed = true;
        }
        if !unsafe { PREFS.motion } {
            w.reveal
                .retarget(w.reveal.target(), now, 0, arena_ui::motion::Easing::Linear);
            w.focus
                .retarget(target, now, 0, arena_ui::motion::Easing::Linear);
        }
        let reveal = w.reveal.sample(now);
        let focus = w.focus.sample(now);
        changed |= reveal != w.reveal_last || focus != w.focus_last;
        w.reveal_last = reveal;
        w.focus_last = focus;
    }
    changed
}
static mut LAST_SCENE: Option<Scene> = None;
/// The presented scene now includes every published region.
fn clear_regions() {
    for s in unsafe { &mut *(&raw mut SESSIONS) } {
        s.regions = compose::Regions::NONE;
        s.popup.regions = compose::Regions::NONE;
    }
    for w in unsafe { &mut *(&raw mut EXTRA_WINDOWS) } {
        w.regions = compose::Regions::NONE;
        w.popup.regions = compose::Regions::NONE;
    }
}

fn reset_installed_applications() {
    unsafe {
        for index in 0..ALL_APPS_CAPACITY {
            INSTALLED_APPS[index] = None;
            INSTALLED_ASSOCIATIONS[index] = [[0; 32]; 8];
            INSTALLED_ASSOCIATION_COUNTS[index] = 0;
        }
        INSTALLED_APP_COUNT = 0;
    }
    for kind in 0..6 {
        let mut display_name = [0u8; 32];
        let name = arena_desktop::apps::TITLES[kind].as_bytes();
        display_name[..name.len()].copy_from_slice(name);
        let (associations, association_count) = builtin_associations(kind);
        unsafe {
            INSTALLED_APPS[kind] = Some(arena_desktop::package::AppEntry {
                application_id: arena_desktop::apps::APPLICATION_IDS[kind],
                display_name,
                flags: arena_desktop::package::manifest::FLAG_MULTI_INSTANCE,
                builtin_kind: kind as u8,
            });
            INSTALLED_ASSOCIATIONS[kind] = associations;
            INSTALLED_ASSOCIATION_COUNTS[kind] = association_count;
        }
    }
    unsafe { INSTALLED_APP_COUNT = 6 };
}

fn refresh_installed_applications() -> Result<(), u64> {
    reset_installed_applications();
    for _ in 0..2 {
        let (count, epoch) = match arena_desktop::package::app_catalog_count() {
            Ok(catalog) => catalog,
            Err(error) => {
                reset_installed_applications();
                unsafe { ALL_APPS_UNAVAILABLE = true };
                return Err(error);
            }
        };
        reset_installed_applications();
        let mut retry = false;
        for index in 0..count {
            match arena_desktop::package::app_catalog_entry(index, epoch) {
                Ok(entry) => {
                    if unsafe { &*(&raw const INSTALLED_APPS) }[..6]
                        .iter()
                        .flatten()
                        .any(|prior| prior.application_id == entry.application_id)
                    {
                        reset_installed_applications();
                        unsafe { ALL_APPS_UNAVAILABLE = true };
                        return Err(PKG_CORRUPT);
                    }
                    let (associations, association_count) =
                        match arena_desktop::package::app_associations(
                            index,
                            &entry.application_id,
                            entry.flags,
                        ) {
                            Ok(metadata) => metadata,
                            Err(PKG_STALE) => {
                                retry = true;
                                break;
                            }
                            Err(error) => {
                                reset_installed_applications();
                                unsafe { ALL_APPS_UNAVAILABLE = true };
                                return Err(error);
                            }
                        };
                    unsafe {
                        INSTALLED_APPS[6 + index] = Some(entry);
                        INSTALLED_ASSOCIATIONS[6 + index] = associations;
                        INSTALLED_ASSOCIATION_COUNTS[6 + index] = association_count as u8;
                    }
                }
                Err(PKG_STALE) => {
                    retry = true;
                    break;
                }
                Err(error) => {
                    reset_installed_applications();
                    unsafe { ALL_APPS_UNAVAILABLE = true };
                    return Err(error);
                }
            }
        }
        if retry {
            reset_installed_applications();
            continue;
        }
        unsafe {
            INSTALLED_APP_COUNT = 6 + count;
            ALL_APPS_UNAVAILABLE = false;
            ALL_APPS_SELECTED = 0;
            ALL_APPS_TOP = 0;
        }
        prune_association_defaults();
        return Ok(());
    }
    reset_installed_applications();
    unsafe { ALL_APPS_UNAVAILABLE = true };
    Err(PKG_STALE)
}

fn open_all_applications() {
    unsafe {
        ALL_APPS_OPEN = true;
        ALL_APPS_OPEN_WITH = false;
        core::ptr::write_bytes(&raw mut ALL_APPS_QUERY, 0, 1);
        ALL_APPS_QUERY_LEN = 0;
        ALL_APPS_SELECTED = 0;
        ALL_APPS_TOP = 0;
    }
    if let Err(error) = refresh_installed_applications() {
        unsafe { ALL_APPS_UNAVAILABLE = true };
        log(b"[desktop] installed application registry unavailable status=");
        log_number(error);
        log(b"\n");
    }
}

fn open_with_document(document: u64, owner: usize, title: [u8; 32], read_only: bool) {
    if document == CAP_NONE {
        return;
    }
    if unsafe { OPEN_WITH_DOCUMENT != CAP_NONE } {
        release_file_cap(document);
        return;
    }
    let content_type = content_type_for_name(&title);
    if !any_handler(content_type) {
        release_file_cap(document);
        desk_notice("NO INSTALLED APPLICATION CAN OPEN THIS FILE");
        return;
    }
    unsafe {
        OPEN_WITH_DOCUMENT = document;
        OPEN_WITH_OWNER = owner;
        OPEN_WITH_TYPE = [0; 32];
        OPEN_WITH_TYPE[..content_type.len()].copy_from_slice(content_type);
        OPEN_WITH_TITLE = title;
        OPEN_WITH_READ_ONLY = read_only;
        ALL_APPS_OPEN = true;
        ALL_APPS_OPEN_WITH = true;
        ALL_APPS_QUERY = [0; 32];
        ALL_APPS_QUERY_LEN = 0;
        ALL_APPS_SELECTED = 0;
        ALL_APPS_TOP = 0;
    }
}

fn close_all_applications() {
    let document = unsafe {
        ALL_APPS_OPEN = false;
        ALL_APPS_OPEN_WITH = false;
        OPEN_WITH_OWNER = usize::MAX;
        core::mem::replace(&mut *(&raw mut OPEN_WITH_DOCUMENT), CAP_NONE)
    };
    release_file_cap(document);
}

fn take_open_with_document() -> u64 {
    unsafe {
        ALL_APPS_OPEN = false;
        ALL_APPS_OPEN_WITH = false;
        OPEN_WITH_OWNER = usize::MAX;
        core::mem::replace(&mut *(&raw mut OPEN_WITH_DOCUMENT), CAP_NONE)
    }
}

fn release_file_cap(cap: u64) {
    if cap == CAP_NONE {
        return;
    }
    if let Some(files) = unsafe { &*(&raw const AFS2) } {
        files.release(cap);
    } else {
        destroy(cap);
    }
}

fn query_matches(name: &[u8; 32], query: &[u8], query_len: usize) -> bool {
    if query_len == 0 {
        return true;
    }
    let length = name
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(name.len());
    if query_len > length {
        return false;
    }
    (0..=length - query_len).any(|start| {
        name[start..start + query_len]
            .iter()
            .zip(query)
            .all(|(a, b)| a.to_ascii_lowercase() == *b)
    })
}

fn filtered_application_index(ordinal: usize) -> Option<usize> {
    let entries = unsafe { &*(&raw const INSTALLED_APPS) };
    let count = unsafe { INSTALLED_APP_COUNT };
    let query = unsafe { &*(&raw const ALL_APPS_QUERY) };
    let query_len = unsafe { ALL_APPS_QUERY_LEN };
    let open_with = unsafe { ALL_APPS_OPEN_WITH };
    let content_type = unsafe { &*(&raw const OPEN_WITH_TYPE) };
    let content_type_len = content_type
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(32);
    let mut matched = 0usize;
    for (index, entry) in entries.iter().take(count).enumerate() {
        let Some(entry) = entry else { continue };
        if query_matches(&entry.display_name, query, query_len) {
            if open_with && !app_handles_type(index, &content_type[..content_type_len]) {
                continue;
            }
            if matched == ordinal {
                return Some(index);
            }
            matched += 1;
        }
    }
    None
}

fn filtered_application_count() -> usize {
    let entries = unsafe { &*(&raw const INSTALLED_APPS) };
    let count = unsafe { INSTALLED_APP_COUNT };
    let query = unsafe { &*(&raw const ALL_APPS_QUERY) };
    let query_len = unsafe { ALL_APPS_QUERY_LEN };
    let open_with = unsafe { ALL_APPS_OPEN_WITH };
    let content_type = unsafe { &*(&raw const OPEN_WITH_TYPE) };
    let content_type_len = content_type
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(32);
    entries
        .iter()
        .take(count)
        .enumerate()
        .filter(|(index, entry)| {
            entry.is_some_and(|entry| query_matches(&entry.display_name, query, query_len))
                && (!open_with || app_handles_type(*index, &content_type[..content_type_len]))
        })
        .count()
}

fn launcher_action(index: usize) -> Action {
    match unsafe { (&*(&raw const INSTALLED_APPS))[index] } {
        Some(app) if app.builtin_kind != u8::MAX => Action::Launch(app.builtin_kind as usize),
        Some(_) => Action::Launch(index),
        None => Action::Changed,
    }
}

fn applications_view() -> Option<arena_desktop::shell::ApplicationsView> {
    if !unsafe { ALL_APPS_OPEN } {
        return None;
    }
    use arena_desktop::shell::{APPLICATION_ROWS, ApplicationsView};
    let total = filtered_application_count();
    let max_top = total.saturating_sub(APPLICATION_ROWS);
    let top = unsafe { ALL_APPS_TOP.min(max_top) };
    let selected = unsafe {
        ALL_APPS_SELECTED
            .saturating_sub(top)
            .min(APPLICATION_ROWS.saturating_sub(1))
    };
    let mut view = ApplicationsView {
        names: [[0; 32]; APPLICATION_ROWS],
        running: [false; APPLICATION_ROWS],
        count: 0,
        selected: selected as u8,
        total,
        query: unsafe { *(&raw const ALL_APPS_QUERY) },
        query_len: unsafe { ALL_APPS_QUERY_LEN as u8 },
        unavailable: unsafe { ALL_APPS_UNAVAILABLE },
        open_with: unsafe { ALL_APPS_OPEN_WITH },
    };
    for row in 0..APPLICATION_ROWS {
        let Some(index) = filtered_application_index(top + row) else {
            break;
        };
        let Some(entry) = (unsafe { (&*(&raw const INSTALLED_APPS))[index] }) else {
            continue;
        };
        view.names[row] = entry.display_name;
        view.running[row] = unsafe { &*(&raw const SESSIONS) }
            .iter()
            .any(|session| session.id != 0 && session.application_id == entry.application_id);
        view.count += 1;
    }
    Some(view)
}

fn move_application_selection(delta: isize) {
    let total = filtered_application_count();
    if total == 0 {
        unsafe {
            ALL_APPS_SELECTED = 0;
            ALL_APPS_TOP = 0
        };
        return;
    }
    let selected = unsafe { ALL_APPS_SELECTED };
    let next = if delta < 0 {
        selected.saturating_sub(delta.unsigned_abs())
    } else {
        selected.saturating_add(delta as usize).min(total - 1)
    };
    let top = if next < unsafe { ALL_APPS_TOP } {
        next
    } else if next >= unsafe { ALL_APPS_TOP } + arena_desktop::shell::APPLICATION_ROWS {
        next + 1 - arena_desktop::shell::APPLICATION_ROWS
    } else {
        unsafe { ALL_APPS_TOP }
    };
    unsafe {
        ALL_APPS_SELECTED = next;
        ALL_APPS_TOP = top;
    }
}

fn applications_key(code: u16, pressed: bool, mods: u8) -> Option<Action> {
    let super_a = mods & wm::MOD_SUPER != 0 && (code == b'a' as u16 || code == b'A' as u16);
    if super_a && pressed {
        if unsafe { ALL_APPS_OPEN } {
            close_all_applications();
        } else {
            open_all_applications();
        }
        return Some(Action::Changed);
    }
    if !unsafe { ALL_APPS_OPEN } {
        return None;
    }
    if !pressed {
        return Some(Action::None);
    }
    match code {
        27 => {
            close_all_applications();
            Some(Action::Changed)
        }
        13 => filtered_application_index(unsafe { ALL_APPS_SELECTED })
            .map(launcher_action)
            .or(Some(Action::Changed)),
        value if value == u16::from(b'd') || value == u16::from(b'D') => {
            save_selected_handler_default();
            Some(Action::Changed)
        }
        258 => {
            move_application_selection(-1);
            Some(Action::Changed)
        }
        259 => {
            move_application_selection(1);
            Some(Action::Changed)
        }
        8 => {
            let length = unsafe { ALL_APPS_QUERY_LEN };
            if length > 0 {
                unsafe {
                    ALL_APPS_QUERY_LEN -= 1;
                    let index = ALL_APPS_QUERY_LEN;
                    ALL_APPS_QUERY[index] = 0;
                    ALL_APPS_SELECTED = 0;
                    ALL_APPS_TOP = 0;
                }
            }
            Some(Action::Changed)
        }
        32..=126 if mods & (wm::MOD_CTRL | wm::MOD_ALT | wm::MOD_SUPER) == 0 => {
            let length = unsafe { ALL_APPS_QUERY_LEN };
            if length < 32 {
                unsafe {
                    ALL_APPS_QUERY[length] = (code as u8).to_ascii_lowercase();
                    ALL_APPS_QUERY_LEN += 1;
                    ALL_APPS_SELECTED = 0;
                    ALL_APPS_TOP = 0;
                }
            }
            Some(Action::Changed)
        }
        _ => Some(Action::None),
    }
}

fn applications_pointer(x: i32, y: i32, buttons: u8, w: i32, h: i32) -> Option<Action> {
    let pressed = buttons & 1 != 0 && unsafe { ALL_APPS_BUTTONS } & 1 == 0;
    unsafe { ALL_APPS_BUTTONS = buttons };
    if !unsafe { ALL_APPS_OPEN } {
        let button = arena_desktop::shell::applications_button_region();
        if pressed
            && x >= button.x
            && y >= button.y
            && x < button.x + button.width as i32
            && y < button.y + button.height as i32
        {
            open_all_applications();
            return Some(Action::Changed);
        }
        return None;
    }
    if pressed {
        let area = arena_desktop::shell::applications_region(w, h);
        if x >= area.x
            && y >= area.y
            && x < area.x + area.width as i32
            && y < area.y + area.height as i32
        {
            if let Some(row) = arena_desktop::shell::applications_row(w, h, x, y) {
                let ordinal = unsafe { ALL_APPS_TOP } + row;
                if let Some(index) = filtered_application_index(ordinal) {
                    unsafe { ALL_APPS_SELECTED = ordinal };
                    if !unsafe { ALL_APPS_OPEN_WITH } {
                        close_all_applications();
                    }
                    return Some(launcher_action(index));
                }
            }
        }
        close_all_applications();
    }
    Some(Action::Changed)
}

/// Descriptive snapshot of everything the compositor draws (see compose.rs).
fn scene(now: u64) -> Scene {
    let state = unsafe { &*(&raw const WM) };
    let sessions = unsafe { &*(&raw const SESSIONS) };
    let extras = unsafe { &*(&raw const EXTRA_WINDOWS) };
    let mut order = [0usize; LIMIT + EXTRA_LIMIT];
    let mut n = 0;
    for (i, s) in sessions.iter().enumerate() {
        // Minimized windows are not drawn (the dock shows them).
        if s.handle != 0 && state.find(s.handle).is_some_and(|w| !w.minimized) {
            order[n] = i;
            n += 1
        }
    }
    for (i, w) in extras.iter().enumerate() {
        if w.handle != 0 && state.find(w.handle).is_some_and(|window| !window.minimized) {
            order[n] = LIMIT + i;
            n += 1;
        }
    }
    order[..n].sort_unstable_by_key(|slot| {
        let handle = if *slot < LIMIT {
            sessions[*slot].handle
        } else {
            extras[*slot - LIMIT].handle
        };
        state.find(handle).map(|window| window.z).unwrap_or(0)
    });
    let mut scene = Scene::EMPTY;
    for (k, slot) in order[..n].iter().copied().enumerate() {
        let (handle, title, reveal, focus, content, regions, published, surface, popup) =
            if slot < LIMIT {
                let s = sessions[slot];
                (
                    s.handle,
                    s.title,
                    s.reveal.sample(now),
                    s.focus.sample(now),
                    s.content,
                    s.regions,
                    s.published,
                    s.surface,
                    s.popup,
                )
            } else {
                let w = extras[slot - LIMIT];
                (
                    w.handle,
                    w.title,
                    w.reveal.sample(now),
                    w.focus.sample(now),
                    w.content,
                    w.regions,
                    w.published,
                    w.surface,
                    w.popup,
                )
            };
        let window = state.find(handle).unwrap_or_else(|| die(88));
        scene.windows[k] = Some(WindowScene {
            slot,
            handle,
            x: window.x,
            y: window.y,
            width: window.width,
            height: window.height,
            reveal: reveal.clamp(0, i32::from(window.height)),
            focus,
            content,
            regions,
            published,
            title,
            surface,
            popup: window.popup.map(|p| {
                let live = popup.handle == p.handle;
                PopupScene {
                    handle: p.handle,
                    x: p.x,
                    y: p.y,
                    width: p.width,
                    height: p.height,
                    content: if live { popup.content } else { 0 },
                    regions: if live {
                        popup.regions
                    } else {
                        compose::Regions::NONE
                    },
                    published: live && popup.published,
                }
            }),
            controls: arena_ui::components::Controls {
                minimize: true,
                maximize: window.resizable,
                maximized: window.restore.is_some() && window.placement == wm::Placement::Maximized,
                hover: match state.hover {
                    Some((h, b)) if h == handle => b as u8,
                    _ => 0,
                },
            },
        });
    }
    scene.count = n;
    let mut running = [0u8; 6];
    let mut minimized = [0u8; 6];
    let mut active = None;
    for s in sessions.iter().filter(|s| s.id != 0) {
        if s.kind < 6 {
            running[s.kind as usize] += 1;
            let mut any = false;
            let mut all_minimized = true;
            for window in state.windows().filter(|window| window.owner == s.id) {
                any = true;
                all_minimized &= window.minimized;
            }
            if any && all_minimized {
                minimized[s.kind as usize] += 1;
            }
            if state
                .focused()
                .and_then(|handle| state.find(handle))
                .is_some_and(|window| window.owner == s.id)
            {
                active = Some(s.kind);
            }
        }
    }
    let switcher = state.switcher().map(|(selected, order, count)| {
        let mut sw = arena_desktop::shell::Switcher {
            count: count as u8,
            selected: selected as u8,
            titles: [[0; 32]; LIMIT],
            kinds: [6; LIMIT],
        };
        for (row, handle) in order[..count].iter().enumerate() {
            if let Some(s) = sessions.iter().find(|s| s.handle == *handle) {
                sw.titles[row] = s.title;
                sw.kinds[row] = s.kind.min(6);
            } else if let Some(w) = extras.iter().find(|w| w.handle == *handle) {
                if let Some(owner) = sessions.get(w.owner_session) {
                    sw.titles[row] = w.title;
                    sw.kinds[row] = owner.kind.min(6);
                }
            }
        }
        sw
    });
    scene.shell = Shell {
        pointer: state.pointer,
        open: state.windows().count(),
        focused_any: state.focused().is_some(),
        running,
        active,
        notice: unsafe { NOTICE }
            .filter(|(_, until)| *until > now)
            .map(|(text, _)| text),
        uptime: now / 1_000_000,
        minimized,
        switcher,
        snap: state.snap_preview().map(|(x, y, w, h)| arena_gfxkit::Rect {
            x,
            y,
            width: u32::from(w),
            height: u32::from(h),
        }),
        chooser: chooser_view(),
        applications: applications_view(),
        desk: {
            desk_watched();
            unsafe { (*(&raw const DESK)).view }
        },
    };
    scene.dark = unsafe { PREFS.dark };
    scene
}
/// Recompose and present only the rectangles whose pixels changed since
/// the last presented scene. The scanout backing retains every other pixel.
fn render(ram: u64, w: usize, h: usize, scanout: u64) {
    let started = perf_now();
    let next = scene(arena_desktop::app_client::now());
    let mut damage = Damage::new(w, h);
    match unsafe { LAST_SCENE } {
        Some(prev) => compose::damage(&prev, &next, &mut damage),
        None => damage.full(),
    }
    if damage.is_empty() {
        unsafe { LAST_SCENE = Some(next) };
        clear_regions();
        probe(P_SKIPPED, started);
        return;
    }
    let pixels = unsafe { core::slice::from_raw_parts_mut(ram as *mut u32, w * h) };
    let mut c = Canvas::new(pixels, w, h, w).unwrap_or_else(|_| die(87));
    let sessions = unsafe { &*(&raw const SESSIONS) };
    let extras = unsafe { &*(&raw const EXTRA_WINDOWS) };
    let theme = arena_ui::theme::palette(next.dark);
    for rect in damage.rects() {
        c.set_clip(*rect);
        compose::compose(
            &mut c,
            &next,
            |slot| {
                if slot < LIMIT {
                    snapshot_of(&sessions[slot])
                } else {
                    snapshot_extra(&extras[slot - LIMIT])
                }
            },
            theme,
        );
    }
    probe(P_COMPOSE, started);
    let phase = perf_now();
    // One display call carries up to four rectangles (Phase 11 latency:
    // a synchronous call per rectangle dominated small frames).
    use arena_compositor_model::wire::{Frame as D, PRESENT_RECTS};
    for chunk in damage.rects().chunks(PRESENT_RECTS) {
        let mut rects = [[0u16; 4]; PRESENT_RECTS];
        for (r, d) in rects.iter_mut().zip(chunk) {
            *r = [d.x as u16, d.y as u16, d.width as u16, d.height as u16];
        }
        let frame = D::PresentRects {
            n: chunk.len() as u8,
            rects,
        };
        let (out, b) = display(frame, scanout);
        if out != [0, 0, CAP_NONE] || D::decode(&b) != Ok(frame) {
            die(90)
        }
    }
    unsafe { LAST_SCENE = Some(next) };
    clear_regions();
    probe(P_PRESENT, phase);
    probe(P_RENDER, started);
    if perf::ENABLED && unsafe { KEY_FRAME } != 0 {
        probe(P_KEY2PHOTON, unsafe {
            core::mem::replace(&mut *(&raw mut KEY_FRAME), 0)
        });
    }
    if perf::ENABLED {
        unsafe {
            (*(&raw mut PERF))[P_KPIXELS].add(damage.pixels() / 1000);
            (*(&raw mut PERF))[P_RECTS].add(damage.rects().len() as u64);
        }
    }
}
/// The chooser requests (ADR-0077). Any live session with a window may ask
/// the user: the request itself grants nothing, the user decides, and the
/// grant lands in the session's own lineage. None: not a chooser frame.
fn choice(index: usize, bytes: &mut [u8; 64]) -> Option<Result<u64, i64>> {
    use arena_desktop::service_wire::Frame as S;
    match S::decode(bytes).ok()? {
        S::Choose {
            save,
            read_only,
            name,
        } => Some(chooser_open(index, save, read_only, name).map(|_| 0)),
        S::TakeGrant => {
            let s = unsafe { &mut *(&raw mut SESSIONS).cast::<Session>().add(index) };
            if !s.grant_ready {
                return Some(Err(-2));
            }
            s.grant_ready = false;
            let granted = core::mem::replace(&mut s.grant, CAP_NONE);
            let reply = if granted == CAP_NONE {
                S::TakeGrant
            } else {
                S::Granted {
                    save: s.grant_save,
                    read_only: s.grant_read_only,
                    name: s.grant_title,
                }
            };
            match reply.encode() {
                Ok(b) => {
                    *bytes = b;
                    unsafe { REPLY_CAP = granted };
                    Some(Ok(0))
                }
                Err(_) => {
                    destroy(granted);
                    Some(Err(-2))
                }
            }
        }
        _ => None,
    }
}
/// Function policy is based on held object/rights and provisioned scope.
/// Caller-supplied startup kind, name, PID and window handle grant nothing.
fn service(index: usize, rights: u64, bytes: &mut [u8; 64]) -> Result<u64, i64> {
    use arena_desktop::{
        scope::{self, Operation as O},
        service_wire::Frame as S,
    };
    let session = unsafe { SESSIONS[index] };
    let request = S::decode(bytes).map_err(|_| -2)?;
    if let Some(r) = choice(index, bytes) {
        return r;
    }
    let operation = match request {
        S::List { .. } => O::List,
        S::Read { .. } => O::Read,
        S::Put { .. } | S::Create { .. } => O::Put,
        S::Delete { .. } => O::Delete,
        S::Configure { .. } => O::Configure,
        S::Launch { .. } => O::Launch,
        _ => return Err(-2),
    };
    if !scope::permits(session.scope, rights, operation) {
        return Err(-2);
    }
    let fs = unsafe { (&mut *(&raw mut FILES)).as_mut() }.ok_or(STATUS_SERVICE_GONE)?;
    match request {
        S::List { mut cursor } => {
            for _ in 0..32 {
                let row = fs.list(cursor)?;
                if row.next == FS_CURSOR_END || scope::public_name(&row.name) {
                    *bytes = S::Entry {
                        cursor: row.next,
                        size: row.size,
                        name: row.name,
                    }
                    .encode()
                    .map_err(|_| -2)?;
                    return Ok(0);
                }
                if row.next <= cursor {
                    return Err(-2);
                }
                cursor = row.next;
            }
            Err(-2)
        }
        S::Read { name } => {
            if !scope::public_name(&name) {
                return Err(-2);
            }
            unsafe { fs.read(name, session.va as *mut u8, 4096) }.map(|n| n as u64)
        }
        S::Put { name, length } => {
            if !scope::public_name(&name) {
                return Err(-2);
            }
            unsafe { fs.put(name, session.va as *const u8, length as usize) }
                .map(|_| u64::from(length))
        }
        S::Create { name } => {
            if !scope::public_name(&name) {
                return Err(-2);
            }
            fs.create(name).map(|_| 0)
        }
        S::Delete { name } => {
            if !scope::public_name(&name) {
                return Err(-2);
            }
            fs.delete(name).map(|_| 0)
        }
        S::Configure { theme, motion } => {
            let p = arena_desktop::preferences::Preferences {
                dark: theme == 1,
                motion,
            };
            let record = p.encode();
            let mut name = [0u8; 32];
            name[..10].copy_from_slice(b"ui10-prefs");
            unsafe { fs.put(name, record.as_ptr(), record.len()) }?;
            unsafe {
                PREFS = p;
                WAKE_ALL = true;
            }
            Ok(0)
        }
        S::Launch { kind, path } => {
            if !scope::can_launch(session.launch_targets, kind) {
                return Err(-2);
            }
            // Offers are stored on their authenticated Desktop session, so a
            // second app cannot race a launch and consume another app's File.
            let offer = unsafe {
                core::mem::replace(&mut (*(&raw mut SESSIONS))[index].offered_file, CAP_NONE)
            };
            let document = match unsafe { &*(&raw const AFS2) } {
                Some(f)
                    if offer != CAP_NONE
                        && kind == 2
                        && session.files_head != CAP_NONE
                        && f.same_lineage(session.files_head, offer) =>
                {
                    offer
                }
                _ => CAP_NONE,
            };
            let r = launch(kind, path, document).map(|_| 0);
            if offer != CAP_NONE {
                if let Some(f) = unsafe { &*(&raw const AFS2) } {
                    f.release(offer);
                } else {
                    destroy(offer);
                }
            }
            r
        }
        _ => Err(-2),
    }
}

/// A capability the current request's reply transfers (a copy; the
/// broker drops its own after replying).
static mut REPLY_CAP: u64 = CAP_NONE;
fn reply(status: u64, result: u64, bytes: &[u8; 64]) {
    let cap = unsafe { core::mem::replace(&mut *(&raw mut REPLY_CAP), CAP_NONE) };
    let rc = unsafe {
        syscall5(
            SYS_IPC_REPLY_CHECKED,
            SERVER,
            status,
            if status == 0 { result } else { 0 },
            if status == 0 { cap } else { CAP_NONE },
            bytes.as_ptr() as u64,
        )
    };
    destroy(cap);
    // A cancelled application call is ordinary liveness, not compositor death.
    // The checked operation consumes the exact abandoned-call tombstone.
    if rc != 0 && rc != STATUS_CALLER_GONE {
        die(91)
    }
}
arena_desktop::entry!(main, 64 * 1024);
extern "C" fn main() -> ! {
    let (mode, b) = display(arena_compositor_model::wire::Frame::Mode, CAP_NONE);
    let w = (mode[1] & 0xffff_ffff) as usize;
    let h = (mode[1] >> 32) as usize;
    let scanout = mode[2];
    let mut bound = [0u64; 2];
    if mode[0] != 0
        || arena_compositor_model::wire::Frame::decode(&b)
            != Ok(arena_compositor_model::wire::Frame::Mode)
        || unsafe {
            syscall6(
                SYS_SHARED_INFO,
                scanout,
                bound.as_mut_ptr() as u64,
                0,
                0,
                0,
                0,
            )
        } != 0
        || w * h * 4 > bound[1] as usize * 4096
    {
        die(92)
    }
    unsafe { (&mut *(&raw mut WM)).configure_screen(w as u16, h as u16, 6) }
        .unwrap_or_else(|_| die(93));
    let (max_w, max_h) = unsafe { (*(&raw const WM)).max_surface() };
    let surface_pages = (u64::from(max_w) * u64::from(max_h) * 4).div_ceil(4096);
    unsafe {
        RESERVE = Reserve {
            surface_pages,
            shared_pages: 1 + surface_pages + TRANSIENT_PAGES + FILE_PAGES,
            snapshot_pages: surface_pages + TRANSIENT_PAGES,
        };
    }
    log(b"[desktop] session reservation shared/snapshot pages=");
    log_number(unsafe { RESERVE.shared_pages });
    log(b"/");
    log_number(unsafe { RESERVE.snapshot_pages });
    log(b"\n");
    let ram = unsafe { syscall2(SYS_SHARED_MAP, scanout, 1) };
    if ram <= 0 {
        die(94)
    }
    let expected = describe(INPUT).unwrap_or_else(|| die(95));
    audit_session_clocks();
    let mut fs = match arena_desktop::fs_backend::Fs::start(13) {
        Ok(fs) => fs,
        Err(rc) => {
            log(b"[desktop] fs backend start refused status=");
            log_number(rc.unsigned_abs());
            log(b"\n");
            die(79)
        }
    };
    let mut pref_name = [0u8; 32];
    pref_name[..10].copy_from_slice(b"ui10-prefs");
    let mut pref_bytes = [0u8; 16];
    if let Ok(n) = unsafe { fs.read(pref_name, pref_bytes.as_mut_ptr(), 16) } {
        if let Some(p) = arena_desktop::preferences::Preferences::decode(&pref_bytes[..n]) {
            unsafe {
                PREFS = p;
            }
        }
    }
    unsafe {
        FILES = Some(fs);
    }
    start_afs2();
    load_association_defaults();
    if let Err(error) = refresh_installed_applications() {
        log(b"[desktop] installed application registry unavailable at boot status=");
        log_number(error);
        log(b"\n");
    }
    unsafe { SHARED_CAP_BASELINE = held_cap_kind_count(7) };
    desk_load(w as i32, h as i32);

    // Requests and input arrive on SERVER; queue them onto CLOCK when the
    // compositor is not parked in RECV, so it never polls (ADR-0071).
    if unsafe { syscall6(SYS_ENDPOINT_BIND, SERVER, CLOCK, BADGE_REQUEST, 0, 0, 0) } != 0 {
        die(98)
    }
    render(ram as u64, w, h, scanout);
    log(b"[desktop] real desktop frame presented; gallery launcher available\n");
    snapshot(true);
    let mut pending_render = false;
    loop {
        // App writers publish ring state before notifying CLOCK. Drain before
        // process-exit sweeping so final bytes are visible before exact
        // ProcessGroup and stream-region teardown.
        drain_standard_streams();
        // A pending Desktop watch is handled by the next scene (render).
        // Keep animation/publication dirtiness across internal badge-bootstrap
        // traffic and first-frame setup; the nonblocking receive loop drains
        // the burst before presenting one combined frame.
        let pending = core::mem::replace(&mut pending_render, false);
        let mut dirty = pending | sweep() | animate() | unsafe { WATCH_SIGNALLED };
        if unsafe { (*(&raw mut WM)).repeat_tick(arena_desktop::app_client::now()) } {
            if perf::ENABLED {
                unsafe { (*(&raw mut PERF))[P_DELIVERED].add(1) };
            }
            wake_clients();
        }
        let mut request = [0, 0, CAP_NONE, 0];
        let mut bytes = [0; 64];
        let rc = unsafe {
            syscall6(
                SYS_IPC_RECV_BADGED,
                SERVER,
                request.as_mut_ptr() as u64,
                bytes.as_mut_ptr() as u64,
                0,
                0,
                0,
            )
        };
        if rc == STATUS_BUSY {
            let tail = perf_now();
            if dirty {
                render(ram as u64, w, h, scanout);
                snapshot(false);
            }
            perf_report();
            let slept = perf_now();
            let now = arena_desktop::app_client::now();
            let due = next_deadline(now);
            let timer = if due == NO_DEADLINE {
                -1
            } else {
                let t = unsafe {
                    syscall3(
                        SYS_TIMER_ARM,
                        CLOCK,
                        BADGE_TIMER,
                        due.saturating_sub(now).max(1),
                    )
                };
                if t < 0 {
                    die(96)
                }
                t
            };
            probe(P_TAIL, tail);
            let woke = unsafe { syscall1(SYS_WAIT, CLOCK) };
            if woke < 0 {
                die(96)
            }
            if woke as u64 & BADGE_WATCH != 0 {
                unsafe { WATCH_SIGNALLED = true };
            }
            // An event (not the timer) woke us: retire the still-armed
            // timer so wakes never accumulate in the bounded table.
            if timer >= 0 {
                let _ = unsafe { syscall1(SYS_TIMER_CANCEL, timer as u64) };
            }
            probe(P_SLEEP, slept);
            continue;
        }
        if rc != 0 {
            die(97)
        }
        let received = perf_now();
        let mut input_request = false;
        let mut landed = request[2];
        let badge = request[3] as u32;
        let mut description = describe(landed);
        let mut status = 2;
        let mut result = 0;
        let mut badge_rejected = false;
        let mut badge_bootstrap = false;
        let mut authenticated_session = None;
        if badge != 0 {
            let sessions = unsafe { &*(&raw const SESSIONS) };
            let badges: [u32; LIMIT] = core::array::from_fn(|index| sessions[index].badge);
            let selected =
                arena_desktop::session_auth::select_live_badge(&badges, badge, |index| {
                    sessions[index]
                        .process
                        .is_some_and(|handle| child_live(index, handle))
                });
            if let Ok(Some(i)) = selected {
                authenticated_session = Some(i);
                let session = unsafe { SESSIONS[i] };
                match (
                    description,
                    arena_desktop::service_wire::Frame::decode(&bytes),
                ) {
                    (None, Ok(arena_desktop::service_wire::Frame::Bootstrap))
                        if session.kind <= 6 =>
                    {
                        let prefs = unsafe { PREFS };
                        bytes = arena_desktop::service_wire::Frame::Started {
                            kind: session.kind,
                            theme: u8::from(prefs.dark),
                            motion: prefs.motion,
                            path: session.path,
                        }
                        .encode()
                        .unwrap_or_else(|_| die(78));
                        status = 0;
                        if session.kind == 6 {
                            log(b"[desktop] installed app ABI-v2 bootstrap replied\n");
                        }
                        badge_bootstrap = true;
                    }
                    (None, _) => {
                        // Internal dispatch context only: the session and
                        // function rights came from the matched badge record,
                        // not from caller bytes or a numeric SharedRegion ID.
                        description = Some([7, session.id, session.function_rights]);
                    }
                    (Some([7, id, rights]), _)
                        if id == session.id
                            && rights == (RIGHTS_READ | RIGHTS_WRITE | RIGHTS_COPY) => {}
                    (Some([12, id, rights]), Ok(arena_desktop::service_wire::Frame::Offer))
                        if session.kind == 1
                            && describe(USER_ROOT).is_some_and(|root| root[1] == id)
                            && rights & (RIGHTS_WRITE | RIGHTS_COPY)
                                == (RIGHTS_WRITE | RIGHTS_COPY) => {}
                    (
                        Some([12, id, rights]),
                        Ok(
                            arena_desktop::service_wire::Frame::OpenDocument { .. }
                            | arena_desktop::service_wire::Frame::OpenWith { .. },
                        ),
                    ) if session.kind == 1
                        && describe(USER_ROOT).is_some_and(|root| root[1] == id)
                        && rights & (RIGHTS_WRITE | RIGHTS_COPY)
                            == (RIGHTS_WRITE | RIGHTS_COPY) => {}
                    _ => badge_rejected = true,
                }
            } else {
                // Unknown, retired, or dead-owner badges never fall back to
                // the legacy object-ID dispatcher.
                badge_rejected = true;
            }
        }
        let mut defer_render = badge_bootstrap;
        transient_caps();
        if badge_rejected {
            // Mutation-free fail-closed path; the landed cap is dropped below.
        } else if badge_bootstrap {
            // The bounded startup reply was built from the badge-bound Session.
        } else if badge == 0
            && request[0] == 0
            && request[1] == 0
            && description
                .is_some_and(|d| d[0] == 10 && d[1] == expected[1] && d[2] & RIGHTS_READ != 0)
        {
            if description
                .is_some_and(|d| d[0] == 10 && d[1] == expected[1] && d[2] & RIGHTS_READ != 0)
            {
                if let Ok(f) = input_wire::Frame::decode(&bytes) {
                    input_request = true;
                    // End the mutable policy borrow before lifecycle actions
                    // take their own access to the same static state.
                    let action = {
                        let state = unsafe { &mut *(&raw mut WM) };
                        state.set_time(arena_desktop::app_client::now());
                        let modal = unsafe { (*(&raw const CHOOSER)).is_some() };
                        match f {
                            // The trusted chooser is modal: presses go to it
                            // only; releases still reach the window policy so
                            // no key stays held (and repeats) behind it.
                            input_wire::Frame::Key {
                                code,
                                pressed: true,
                                ..
                            } if modal => {
                                chooser_key(code);
                                Action::Changed
                            }
                            input_wire::Frame::Key {
                                code,
                                pressed,
                                mods,
                            } => {
                                let app_action = if modal {
                                    None
                                } else {
                                    applications_key(code, pressed, mods)
                                };
                                let before = state.delivered;
                                let a = app_action
                                    .unwrap_or_else(|| state.key_input(code, pressed, mods));
                                if app_action.is_none()
                                    && pressed
                                    && !modal
                                    && let (Some(handle), Some(byte)) =
                                        (state.focused(), stream_keyboard_byte(code, mods))
                                    && let Some(owner) = stream_owner_for_window(handle)
                                {
                                    queue_stdin_byte(owner, byte);
                                }
                                if perf::ENABLED {
                                    let p = unsafe { &mut *(&raw mut PERF) };
                                    if app_action.is_none() && pressed && unsafe { KEY_AT } == 0 {
                                        unsafe { (KEY_AT, KEY_POLLED) = (perf_now(), false) };
                                    }
                                    p[if pressed { P_KEYDOWN } else { P_KEYUP }]
                                        .add(u64::from(code));
                                    for _ in before..state.delivered {
                                        p[P_DELIVERED].add(0);
                                    }
                                }
                                // With no window focused, Enter, Delete and
                                // Esc act on the desktop's selected icons.
                                if app_action.is_none()
                                    && pressed
                                    && state.focused().is_none()
                                    && matches!(code, 13 | 27 | 262)
                                    && let Some(mut store) = desk_store()
                                {
                                    let desk = unsafe { &mut *(&raw mut DESK) };
                                    desk_effect(desk.key(&mut store, code));
                                    Action::Changed
                                } else {
                                    a
                                }
                            }
                            input_wire::Frame::Pointer {
                                x,
                                y,
                                buttons,
                                wheel,
                            } => {
                                let (px, py) = (
                                    (u32::from(x) * (w as u32 - 1) / 32767) as i32,
                                    (u32::from(y) * (h as u32 - 1) / 32767) as i32,
                                );
                                if modal {
                                    chooser_pointer(px, py, buttons, w as i32, h as i32);
                                }
                                let app_action = if modal {
                                    None
                                } else {
                                    applications_pointer(px, py, buttons, w as i32, h as i32)
                                };
                                if app_action.is_some() {
                                    state.pointer = (px, py);
                                }
                                // The desktop surface takes presses on bare
                                // desktop and everything while it holds a
                                // press or its menu; the window policy then
                                // sees the pointer without buttons.
                                let desk = unsafe { &mut *(&raw mut DESK) };
                                let pressing = buttons & 3 != 0 && unsafe { DESK_BUTTONS } & 3 == 0;
                                unsafe { DESK_BUTTONS = buttons };
                                let to_desk = app_action.is_none()
                                    && !modal
                                    && desk_store().is_some()
                                    && (desk.busy() || (pressing && state.bare(px, py)));
                                if app_action.is_none() && !to_desk {
                                    desk.track(buttons);
                                }
                                if to_desk && let Some(mut store) = desk_store() {
                                    if pressing && !desk.busy() {
                                        state.blur();
                                    }
                                    let ctrl = state.mods & arena_desktop::model::MOD_CTRL != 0;
                                    let now = arena_desktop::app_client::now();
                                    let e = desk.pointer(&mut store, px, py, buttons, ctrl, now);
                                    desk_effect(e);
                                }
                                let a = app_action.unwrap_or_else(|| {
                                    let action = state.pointer(
                                        px,
                                        py,
                                        if modal || to_desk { 0 } else { buttons },
                                    );
                                    if modal || to_desk {
                                        Action::Changed
                                    } else {
                                        action
                                    }
                                });
                                if wheel != 0 && app_action.is_none() {
                                    state.wheel(wheel);
                                }
                                a
                            }
                        }
                    };
                    match action {
                        Action::Launch(kind)
                            if kind < 6
                                && !unsafe { ALL_APPS_OPEN_WITH }
                                && restore_minimized(kind as u8) =>
                        {
                            dirty = true;
                        }
                        Action::Launch(selection) => {
                            let full_sessions =
                                unsafe { (&*(&raw const SESSIONS)).iter().all(|s| s.id != 0) };
                            let before = full_sessions
                                .then(|| (cap_inventory_snapshot(), observe_receipt()));
                            let open_with = unsafe { ALL_APPS_OPEN_WITH };
                            let result = if open_with {
                                let document = take_open_with_document();
                                let title = unsafe { *(&raw const OPEN_WITH_TITLE) };
                                let read_only = unsafe { OPEN_WITH_READ_ONLY };
                                launch_registry_entry_with_title(
                                    selection, document, title, read_only,
                                )
                            } else if selection < 6 {
                                close_all_applications();
                                launch(selection as u8, [0; 32], CAP_NONE)
                            } else {
                                close_all_applications();
                                match unsafe { (&*(&raw const INSTALLED_APPS))[selection] } {
                                    Some(app) => {
                                        if app.builtin_kind != u8::MAX {
                                            launch(app.builtin_kind, [0; 32], CAP_NONE)
                                        } else {
                                            launch_installed_application(
                                                &app.application_id,
                                                app.flags,
                                                CAP_NONE,
                                                false,
                                            )
                                        }
                                    }
                                    None => Err(PKG_STALE as i64),
                                }
                            };
                            if let Err(rc) = result {
                                if let Some((caps_before, resources_before)) = before {
                                    log_capacity_refusal_inventory(caps_before, resources_before);
                                    snapshot(true);
                                }
                                unsafe {
                                    NOTICE = Some((
                                        "LAUNCH REFUSED / DESKTOP CAPACITY",
                                        arena_desktop::app_client::now() + 3_000_000,
                                    ));
                                }
                                log(b"[desktop] launch refused at bounded capacity (status -");
                                log_number(rc.unsigned_abs());
                                log(b")\n");
                            }
                            dirty = true;
                        }
                        Action::Close(handle) => {
                            if let Some(i) = unsafe { &*(&raw const SESSIONS) }
                                .iter()
                                .position(|s| s.handle == handle)
                            {
                                if unsafe { SESSIONS[i].close_pending } {
                                    retire_primary_window(i)
                                } else {
                                    let _ = unsafe {
                                        (&mut *(&raw mut WM))
                                            .send(handle, arena_desktop::model::Event::Close)
                                    };
                                    unsafe {
                                        SESSIONS[i].close_pending = true;
                                    }
                                }
                            } else if let Some(wi) = extra_index(handle) {
                                let owner = unsafe { EXTRA_WINDOWS[wi].owner_session };
                                if unsafe { EXTRA_WINDOWS[wi].close_pending } {
                                    retire(owner, true)
                                } else {
                                    let _ = unsafe {
                                        (&mut *(&raw mut WM))
                                            .send(handle, arena_desktop::model::Event::Close)
                                    };
                                    unsafe { EXTRA_WINDOWS[wi].close_pending = true };
                                }
                            }
                            dirty = true;
                        }
                        Action::Changed => dirty = true,
                        Action::None => {}
                    }
                    status = 0;
                }
            }
        } else if description.is_some_and(|d| d[0] == 1 && d[2] & RIGHTS_READ != 0)
            && arena_desktop::service_wire::Frame::decode(&bytes)
                == Ok(arena_desktop::service_wire::Frame::LaunchImage)
        {
            match launch_image(
                landed,
                255,
                0,
                RIGHTS_READ | RIGHTS_COPY,
                [0; 32],
                false,
                0,
                CAP_NONE,
            ) {
                Ok(()) => {
                    status = 0;
                    dirty = true;
                }
                Err(e) => status = e as u64,
            }
        } else if let (Some(index), Some([12, _, _]), Ok(frame)) = (
            authenticated_session,
            description,
            arena_desktop::service_wire::Frame::decode(&bytes),
        ) {
            let session = unsafe { SESSIONS[index] };
            let offered = match frame {
                arena_desktop::service_wire::Frame::OpenDocument { name } if session.kind == 1 => {
                    Some((name, false, false))
                }
                arena_desktop::service_wire::Frame::OpenWith { name } if session.kind == 1 => {
                    Some((name, true, true))
                }
                _ => None,
            };
            if let Some((name, force_chooser, read_only)) = offered
                && session.files_head != CAP_NONE
                && unsafe { &*(&raw const AFS2) }
                    .is_some_and(|files| files.same_lineage(session.files_head, landed))
            {
                open_document_with_registry(landed, name, force_chooser, index, read_only);
                landed = CAP_NONE;
                status = 0;
                dirty = true;
            }
        } else if description
            .is_some_and(|d| d[0] == 12 && describe(USER_ROOT).is_some_and(|r| r[1] == d[1]))
            && arena_desktop::service_wire::Frame::decode(&bytes)
                == Ok(arena_desktop::service_wire::Frame::Offer)
        {
            // Legacy Offer frames are bound to their authenticated Files
            // session and cannot replace another session's pending File.
            if let Some(index) = authenticated_session
                && unsafe { SESSIONS[index].kind == 1 }
                && unsafe { &*(&raw const AFS2) }.is_some_and(|files| {
                    files.same_lineage(unsafe { SESSIONS[index].files_head }, landed)
                })
            {
                let old = unsafe {
                    core::mem::replace(&mut (*(&raw mut SESSIONS))[index].offered_file, landed)
                };
                if old != CAP_NONE {
                    release_file_cap(old);
                }
                landed = CAP_NONE;
                status = 0;
            }
        } else if let (Some(index), Ok(frame)) = (
            authenticated_session,
            arena_desktop::service_wire::Frame::decode(&bytes),
        ) && matches!(
            frame,
            arena_desktop::service_wire::Frame::SpawnHelper { .. }
                | arena_desktop::service_wire::Frame::WaitHelper { .. }
                | arena_desktop::service_wire::Frame::TerminateHelper { .. }
        ) {
            let session = unsafe { SESSIONS[index] };
            if landed == CAP_NONE
                && session.kind == 6
                && description == Some([7, session.id, session.function_rights])
            {
                match helper_service(index, &mut bytes) {
                    Ok(value) => {
                        status = 0;
                        result = value;
                    }
                    Err(error) => status = error as u64,
                }
            } else {
                status = STATUS_BAD_ARG as u64;
            }
        } else if let Some([7, id, rights]) = description {
            if let Some(i) = unsafe { &*(&raw const SESSIONS) }
                .iter()
                .enumerate()
                .position(|(index, s)| {
                    s.id == id && s.process.is_some_and(|handle| child_live(index, handle))
                })
            {
                if rights & RIGHTS_DESTROY != 0 {
                    match service(i, rights, &mut bytes) {
                        Ok(value) => {
                            status = 0;
                            result = value;
                            dirty = true;
                        }
                        Err(error) => {
                            status = error as u64;
                        }
                    }
                }
            }
            if rights & (RIGHTS_READ | RIGHTS_WRITE | RIGHTS_COPY)
                == (RIGHTS_READ | RIGHTS_WRITE | RIGHTS_COPY)
            {
                if let Some(i) = unsafe { &*(&raw const SESSIONS) }
                    .iter()
                    .enumerate()
                    .position(|(index, s)| {
                        s.id == id && s.process.is_some_and(|handle| child_live(index, handle))
                    })
                {
                    if arena_desktop::service_wire::Frame::decode(&bytes)
                        == Ok(arena_desktop::service_wire::Frame::Bootstrap)
                    {
                        let s = unsafe { SESSIONS[i] };
                        let p = unsafe { PREFS };
                        if s.kind <= 6 {
                            bytes = arena_desktop::service_wire::Frame::Started {
                                kind: s.kind,
                                theme: u8::from(p.dark),
                                motion: p.motion,
                                path: s.path,
                            }
                            .encode()
                            .unwrap_or_else(|_| die(78));
                            status = 0;
                        }
                    }
                    if arena_desktop::service_wire::Frame::decode(&bytes)
                        == Ok(arena_desktop::service_wire::Frame::Display)
                    {
                        result = w as u64 | ((h as u64) << 32);
                        status = 0;
                    }
                    // Ordinary graphical sessions (signed applications
                    // included) may ask the user through the chooser.
                    if rights & RIGHTS_DESTROY == 0 {
                        if let Some(r) = choice(i, &mut bytes) {
                            match r {
                                Ok(v) => {
                                    status = 0;
                                    result = v;
                                    dirty = true;
                                }
                                Err(e) => status = e as u64,
                            }
                        }
                    }
                    if let Ok(f) = Frame::decode(&bytes) {
                        let state = unsafe { &mut *(&raw mut WM) };
                        let s = unsafe { &mut *(&raw mut SESSIONS).cast::<Session>().add(i) };
                        match f {
                            Frame::Create { width, height }
                                if s.handle == 0
                                    && u64::from(width) * u64::from(height)
                                        <= unsafe { RESERVE.surface_pages } * 1024 =>
                            {
                                if let Ok(handle) = state.create(id, width, height) {
                                    s.handle = handle;
                                    s.surface = (width, height);
                                    s.reveal.retarget(
                                        i32::from(height),
                                        arena_desktop::app_client::now(),
                                        if unsafe { PREFS.motion } {
                                            arena_ui::motion::OPEN_US
                                        } else {
                                            0
                                        },
                                        arena_ui::motion::Easing::Smooth,
                                    );
                                    result = handle;
                                    status = 0;
                                    defer_render = true;
                                }
                            }
                            Frame::CreateAdditional { width, height }
                                if u64::from(width) * u64::from(height)
                                    <= unsafe { RESERVE.surface_pages } * 1024 =>
                            {
                                match create_extra_window(state, i, id, width, height) {
                                    Ok(handle) => {
                                        result = handle;
                                        status = 0;
                                        defer_render = true;
                                    }
                                    Err(error) => status = error as u64,
                                }
                            }
                            Frame::Title { handle, text } if state.owned(id, handle) => {
                                if s.handle == handle {
                                    s.title = text;
                                    if s.published {
                                        dirty = true;
                                    } else {
                                        defer_render = true;
                                    }
                                    status = 0;
                                } else if let Some(wi) = extra_index(handle)
                                    && unsafe { EXTRA_WINDOWS[wi].owner_session == i }
                                {
                                    unsafe { EXTRA_WINDOWS[wi].title = text };
                                    if unsafe { EXTRA_WINDOWS[wi].published } {
                                        dirty = true;
                                    } else {
                                        defer_render = true;
                                    }
                                    status = 0;
                                }
                            }
                            Frame::Damage { handle, rects } if state.owned(id, handle) => {
                                let extra = if s.handle != handle {
                                    extra_index(handle).filter(|wi| unsafe {
                                        EXTRA_WINDOWS[*wi].owner_session == i
                                    })
                                } else {
                                    None
                                };
                                if s.handle == handle || extra.is_some() {
                                    let copy_started = perf_now();
                                    if perf::ENABLED && unsafe { KEY_AT } != 0 {
                                        probe(P_KEY2DAMAGE, unsafe { KEY_AT });
                                        unsafe {
                                            KEY_FRAME =
                                                core::mem::replace(&mut *(&raw mut KEY_AT), 0)
                                        };
                                    }
                                    let (ww, wh, source, snapshot, was_published) =
                                        if let Some(wi) = extra {
                                            let window = unsafe { EXTRA_WINDOWS[wi] };
                                            (
                                                usize::from(window.surface.0),
                                                usize::from(window.surface.1),
                                                window.va + PIXEL_OFFSET as u64,
                                                window.snapshot,
                                                window.published,
                                            )
                                        } else {
                                            (
                                                usize::from(s.surface.0),
                                                usize::from(s.surface.1),
                                                s.va + PIXEL_OFFSET as u64,
                                                s.snapshot,
                                                s.published,
                                            )
                                        };
                                    // Every declared rectangle must lie inside this
                                    // session's published surface; otherwise nothing
                                    // is published at all.
                                    let inside = rects.rects().iter().all(|&[x, y, w, h]| {
                                        x as usize + w as usize <= ww
                                            && y as usize + h as usize <= wh
                                    });
                                    if inside {
                                        let full = [[0, 0, ww as u16, wh as u16]];
                                        let list = if rects.n == 0 {
                                            &full[..]
                                        } else {
                                            rects.rects()
                                        };
                                        publish_rects(source, snapshot, ww, list);
                                        if let Some(wi) = extra {
                                            let window = unsafe {
                                                &mut *(&raw mut EXTRA_WINDOWS)
                                                    .cast::<ExtraWindow>()
                                                    .add(wi)
                                            };
                                            if rects.n == 0 || !was_published {
                                                window.regions.mark_full();
                                            } else {
                                                for r in rects.rects() {
                                                    window.regions.add(*r);
                                                }
                                            }
                                            window.published = true;
                                            window.content = window.content.wrapping_add(1);
                                        } else {
                                            if rects.n == 0 || !was_published {
                                                s.regions.mark_full();
                                            } else {
                                                for r in rects.rects() {
                                                    s.regions.add(*r);
                                                }
                                            }
                                            s.published = true;
                                            s.content = s.content.wrapping_add(1);
                                        }
                                        probe(P_DAMAGE, copy_started);
                                        status = 0;
                                        dirty = true;
                                    }
                                }
                            }
                            // Publication of the session's own transient surface.
                            Frame::Damage { handle, rects } if state.popup_owned(id, handle) => {
                                let (parent, p) =
                                    state.find_popup(handle).unwrap_or_else(|| die(88));
                                let parent_handle = parent.handle;
                                let extra = if s.handle != parent_handle {
                                    extra_index(parent_handle).filter(|wi| unsafe {
                                        EXTRA_WINDOWS[*wi].owner_session == i
                                    })
                                } else {
                                    None
                                };
                                let popup_state = if s.handle == parent_handle {
                                    s.popup
                                } else if let Some(wi) = extra {
                                    unsafe { EXTRA_WINDOWS[wi].popup }
                                } else {
                                    NO_POPUP
                                };
                                if popup_state.handle == handle {
                                    let (pw, ph) = (usize::from(p.width), usize::from(p.height));
                                    let inside = rects.rects().iter().all(|&[x, y, w, h]| {
                                        x as usize + w as usize <= pw
                                            && y as usize + h as usize <= ph
                                    });
                                    if inside {
                                        let reserve = unsafe { RESERVE };
                                        let full = [[0, 0, pw as u16, ph as u16]];
                                        let list = if rects.n == 0 {
                                            &full[..]
                                        } else {
                                            rects.rects()
                                        };
                                        publish_rects(
                                            (if let Some(wi) = extra {
                                                unsafe { EXTRA_WINDOWS[wi].va }
                                            } else {
                                                s.va
                                            }) + (reserve.shared_pages
                                                - TRANSIENT_PAGES
                                                - FILE_PAGES)
                                                * 4096,
                                            (if let Some(wi) = extra {
                                                unsafe { EXTRA_WINDOWS[wi].snapshot }
                                            } else {
                                                s.snapshot
                                            }) + reserve.surface_pages * 4096,
                                            pw,
                                            list,
                                        );
                                        if let Some(wi) = extra {
                                            let popup = unsafe {
                                                &mut *(&raw mut EXTRA_WINDOWS)
                                                    .cast::<ExtraWindow>()
                                                    .add(wi)
                                            };
                                            if rects.n == 0 || !popup.popup.published {
                                                popup.popup.regions.mark_full();
                                            } else {
                                                for r in rects.rects() {
                                                    popup.popup.regions.add(*r);
                                                }
                                            }
                                            popup.popup.published = true;
                                            popup.popup.content =
                                                popup.popup.content.wrapping_add(1);
                                        } else {
                                            if rects.n == 0 || !s.popup.published {
                                                s.popup.regions.mark_full();
                                            } else {
                                                for r in rects.rects() {
                                                    s.popup.regions.add(*r);
                                                }
                                            }
                                            s.popup.published = true;
                                            s.popup.content = s.popup.content.wrapping_add(1);
                                        }
                                        status = 0;
                                        dirty = true;
                                    }
                                }
                            }
                            // The client publishes its whole surface at exactly
                            // the size the window policy configured.
                            Frame::Resize {
                                handle,
                                width,
                                height,
                            } if state.owned(id, handle) => {
                                let window = state.find(handle).unwrap_or_else(|| die(88));
                                if (width, height) == (window.width, window.height) {
                                    let extra = if s.handle != handle {
                                        extra_index(handle).filter(|wi| unsafe {
                                            EXTRA_WINDOWS[*wi].owner_session == i
                                        })
                                    } else {
                                        None
                                    };
                                    if s.handle == handle || extra.is_some() {
                                        publish_rects(
                                            (if let Some(wi) = extra {
                                                unsafe { EXTRA_WINDOWS[wi].va }
                                            } else {
                                                s.va
                                            }) + PIXEL_OFFSET as u64,
                                            if let Some(wi) = extra {
                                                unsafe { EXTRA_WINDOWS[wi].snapshot }
                                            } else {
                                                s.snapshot
                                            },
                                            usize::from(width),
                                            &[[0, 0, width, height]],
                                        );
                                        if let Some(wi) = extra {
                                            let window = unsafe {
                                                &mut *(&raw mut EXTRA_WINDOWS)
                                                    .cast::<ExtraWindow>()
                                                    .add(wi)
                                            };
                                            window.surface = (width, height);
                                            window.regions.mark_full();
                                            window.published = true;
                                            window.content = window.content.wrapping_add(1);
                                        } else {
                                            s.surface = (width, height);
                                            s.regions.mark_full();
                                            s.published = true;
                                            s.content = s.content.wrapping_add(1);
                                        }
                                        status = 0;
                                        dirty = true;
                                    }
                                }
                            }
                            Frame::Resizable {
                                handle,
                                min_width,
                                min_height,
                            } if state.owned(id, handle) => {
                                if state.set_resizable(handle, min_width, min_height).is_ok() {
                                    status = 0;
                                }
                            }
                            Frame::Popup {
                                handle,
                                kind,
                                x,
                                y,
                                width,
                                height,
                            } if state.owned(id, handle) => {
                                if let Ok(popup) =
                                    state.open_popup(id, handle, kind, x, y, width, height)
                                {
                                    let extra = if s.handle != handle {
                                        extra_index(handle).filter(|wi| unsafe {
                                            EXTRA_WINDOWS[*wi].owner_session == i
                                        })
                                    } else {
                                        None
                                    };
                                    if s.handle == handle {
                                        s.popup = PopupState {
                                            handle: popup,
                                            ..NO_POPUP
                                        };
                                        result = popup;
                                        status = 0;
                                        dirty = true;
                                    } else if let Some(wi) = extra {
                                        unsafe {
                                            EXTRA_WINDOWS[wi].popup = PopupState {
                                                handle: popup,
                                                ..NO_POPUP
                                            }
                                        };
                                        result = popup;
                                        status = 0;
                                        dirty = true;
                                    }
                                }
                            }
                            Frame::Dismiss { handle } if state.popup_owned(id, handle) => {
                                let parent_handle =
                                    state.find_popup(handle).unwrap_or_else(|| die(88)).0.handle;
                                if state.close_popup(id, handle).is_ok() {
                                    if s.handle == parent_handle {
                                        s.popup = NO_POPUP;
                                        status = 0;
                                        dirty = true;
                                    } else if let Some(wi) = extra_index(parent_handle)
                                        && unsafe { EXTRA_WINDOWS[wi].owner_session == i }
                                    {
                                        unsafe { EXTRA_WINDOWS[wi].popup = NO_POPUP };
                                        status = 0;
                                        dirty = true;
                                    }
                                }
                            }
                            Frame::Poll { handle } if state.owned(id, handle) => {
                                unsafe { WOKEN[i] = false };
                                if perf::ENABLED && unsafe { KEY_AT != 0 && !KEY_POLLED } {
                                    probe(P_KEY2POLL, unsafe { KEY_AT });
                                    if unsafe { KEY_NOTIFIED } != 0 {
                                        probe(P_NOTIFY2POLL, unsafe { KEY_NOTIFIED });
                                    }
                                    unsafe { (KEY_POLLED, KEY_NOTIFIED) = (true, 0) };
                                }
                                if let Ok(event) = state.poll(handle) {
                                    // Bit 2: more events are queued, so a
                                    // client stops polling when it is clear
                                    // (Phase 11: an empty poll was a whole
                                    // round trip on every key).
                                    result = u64::from(unsafe { PREFS.dark })
                                        | (u64::from(unsafe { PREFS.motion }) << 1)
                                        | (u64::from(state.pending(handle)) << 2);
                                    if let Some(event) = event {
                                        bytes = Frame::Event { handle, event }
                                            .encode()
                                            .unwrap_or_else(|_| die(98));
                                    }
                                    status = 0;
                                }
                            }
                            Frame::CancelClose { handle } if state.owned(id, handle) => {
                                if s.handle == handle {
                                    s.close_pending = false;
                                    status = 0;
                                } else if let Some(wi) = extra_index(handle)
                                    && unsafe { EXTRA_WINDOWS[wi].owner_session == i }
                                {
                                    unsafe { EXTRA_WINDOWS[wi].close_pending = false };
                                    status = 0;
                                }
                            }
                            Frame::DestroyWindow { handle } if state.owned(id, handle) => {
                                if s.handle == handle {
                                    state.retire(handle).unwrap_or_else(|_| die(85));
                                    s.handle = 0;
                                    s.title = [0; 32];
                                    s.published = false;
                                    s.close_pending = false;
                                    s.ending = false;
                                    s.content = 0;
                                    s.regions = compose::Regions::NONE;
                                    s.surface = (0, 0);
                                    s.popup = NO_POPUP;
                                    status = 0;
                                    dirty = true;
                                } else if let Some(wi) = extra_index(handle)
                                    && unsafe { EXTRA_WINDOWS[wi].owner_session == i }
                                {
                                    state.retire(handle).unwrap_or_else(|_| die(85));
                                    let window = unsafe { EXTRA_WINDOWS[wi] };
                                    if unsafe {
                                        syscall6(SYS_SHARED_UNMAP, window.va, 0, 0, 0, 0, 0)
                                    } != 0
                                        || unsafe {
                                            syscall6(
                                                SYS_SHARED_UNMAP,
                                                window.snapshot,
                                                0,
                                                0,
                                                0,
                                                0,
                                                0,
                                            )
                                        } != 0
                                    {
                                        die(86)
                                    }
                                    unsafe { EXTRA_WINDOWS[wi] = EMPTY_EXTRA };
                                    status = 0;
                                    dirty = true;
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        probe(P_HANDLE, received);
        // Wake the clients this request gave events to BEFORE replying:
        // the kernel then queues the reply behind them (ADR-0072 causal
        // rule), so a typed key reaches its application before inputd
        // delivers the key's release (Phase 11 latency).
        let waking = perf_now();
        deliver_chosen();
        wake_clients();
        probe(P_WAKE, waking);
        let replying = perf_now();
        destroy(landed);
        reply(status, result, &bytes);
        probe(P_REPLY, replying);
        probe(P_REQUEST, received);
        if dirty {
            if defer_render {
                // Startup audits, first Create/Title, and the animation they
                // overlap do not need intermediate scanouts. Preserve dirt
                // for the next visible mutation or the empty-queue turn.
                pending_render = true;
            } else {
                render(ram as u64, w, h, scanout)
            }
        }
        if input_request {
            probe(P_INPUT, received);
        }
        if (dirty && (bytes[5] == 1 || description.is_some_and(|d| d[0] == 10)))
            || (bytes[5] == 10 && description.is_some_and(|d| d[0] == 1))
        {
            let logged = perf_now();
            // Lifecycle requests (a client's surface, a launch) always;
            // input-driven frames only when a counter changed.
            snapshot(!input_request);
            probe(P_SNAPSHOT, logged);
        }
    }
}
