//! ADR-0055: two bounded, volatile, cap-gated dynamic executable images.
//! A full monotonic ID names an image; the backing slot is never identity.
//! All mutation is IF=0 on the single CPU. No key or filesystem data lives here.
use crate::cap::{Cap, CapObj};
use crate::elf;
use crate::sched;
use crate::sync::{SyncCell, without_interrupts};

pub const FIRST: u32 = 27;
pub const SLOTS: usize = 16;
pub const MAX_BYTES: usize = 256 * 1024;
pub const MAX_LOAD_PAGES: u64 = 128;
#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Free,
    Reserved,
    Live,
    Revoked,
}
struct Entry {
    bytes: [u8; MAX_BYTES],
    state: State,
    id: u32,
    len: usize,
    kind: elf::ImageKind,
    entry: u64,
    load_base: u64,
    image_end: u64,
    rela_offset: u64,
    rela_count: usize,
    refs: u32,
    pins: u32,
}
impl Entry {
    const EMPTY: Self = Self {
        bytes: [0; MAX_BYTES],
        state: State::Free,
        id: 0,
        len: 0,
        kind: elf::ImageKind::FixedExec,
        entry: 0,
        load_base: 0,
        image_end: 0,
        rela_offset: 0,
        rela_count: 0,
        refs: 0,
        pins: 0,
    };
    fn retire(&mut self) {
        self.bytes.fill(0);
        self.state = State::Free;
        self.id = 0;
        self.len = 0;
        self.kind = elf::ImageKind::FixedExec;
        self.entry = 0;
        self.load_base = 0;
        self.image_end = 0;
        self.rela_offset = 0;
        self.rela_count = 0;
        self.refs = 0;
        self.pins = 0;
    }
}
struct Registry {
    entries: [Entry; SLOTS],
    next_id: u64,
}
static MANAGER_PID: SyncCell<u64> = SyncCell::new(0);
static MANAGER_ALIVE: SyncCell<bool> = SyncCell::new(false);
pub fn set_manager(pid: u64) {
    without_interrupts(|| unsafe {
        assert!(*MANAGER_PID.get() == 0 && pid != 0);
        *MANAGER_PID.get() = pid;
        *MANAGER_ALIVE.get() = true;
    });
}
/// Registrar authority is held by cap *possession* AND the kernel-owned
/// singleton's liveness. Death permanently disables even surviving copies.
pub fn registrar_alive() -> bool {
    without_interrupts(|| unsafe { *MANAGER_ALIVE.get() })
}
/// Fail-stop BEFORE record_exit, notification, IPC, cap or process teardown
/// on every last-thread and pre-destroy path. No pid check is used for
/// REGISTER authorization: this records only the root manager's demise.
pub fn manager_death_check(pid: u64) {
    if !without_interrupts(|| unsafe { *MANAGER_PID.get() == pid }) {
        return;
    }
    if active() || crate::spawn::unretired_dynamic_child() {
        crate::halt::halt_machine("ADR-0055: manager death with dynamic authority/child");
    }
    without_interrupts(|| unsafe { *MANAGER_ALIVE.get() = false });
}

static REG: SyncCell<Registry> = SyncCell::new(Registry {
    entries: [const { Entry::EMPTY }; SLOTS],
    next_id: FIRST as u64,
});

pub fn live(id: u32) -> bool {
    without_interrupts(|| unsafe {
        (*REG.get())
            .entries
            .iter()
            .any(|e| e.state == State::Live && e.id == id)
    })
}

pub fn reserve() -> Option<usize> {
    without_interrupts(|| unsafe {
        let r = &mut *REG.get();
        if r.next_id > u32::MAX as u64 {
            return None;
        }
        let idx = r.entries.iter().position(|e| e.state == State::Free)?;
        r.entries[idx].state = State::Reserved;
        Some(idx)
    })
}

pub fn abandon(idx: usize) {
    without_interrupts(|| unsafe {
        let e = &mut (*REG.get()).entries[idx];
        assert!(e.state == State::Reserved);
        e.retire();
    });
}

/// Caller must page-check the *entire* span first, then STAC/CLAC while
/// copying. RESERVED is not published, so no other path can borrow these
/// bytes until validation and mint have succeeded.
pub unsafe fn copy_from_user(idx: usize, ptr: *const u8, len: usize) {
    without_interrupts(|| unsafe {
        let e = &mut (*REG.get()).entries[idx];
        assert!(e.state == State::Reserved && (1..=MAX_BYTES).contains(&len));
        for i in 0..len {
            e.bytes[i] = core::ptr::read_volatile(ptr.add(i));
        }
        e.len = len;
    });
}

/// Validate the exact one-time copy using the unchanged production ELF
/// parser, and bound real leaf pages, stack and address arithmetic before
/// any ID is allocated. Static embedded images have no new page limit.
pub fn validate_reserved(idx: usize) -> bool {
    without_interrupts(|| unsafe {
        let e = &(*REG.get()).entries[idx];
        if e.state != State::Reserved {
            return false;
        }
        let parsed = match elf::validate(&e.bytes[..e.len]) {
            Ok(parsed) => parsed,
            Err(reason) => {
                crate::log_error!("image", "dynamic Image ELF refused: {reason}");
                return false;
            }
        };
        if parsed.nsegs == 0 || parsed.nsegs + 1 > sched::USER_REGIONS_MAX {
            crate::log_error!("image", "dynamic Image region-count preflight refused");
            return false;
        }
        let mut pages = 0u64;
        let mut top = 0u64;
        let mut load_base = u64::MAX;
        for seg in parsed.segs[..parsed.nsegs].iter() {
            let Some(end) = seg.vaddr.checked_add(seg.memsz) else {
                crate::log_error!("image", "dynamic Image segment-end overflow");
                return false;
            };
            let Some(round) = end.checked_add(4095).map(|v| v & !4095) else {
                crate::log_error!("image", "dynamic Image segment rounding overflow");
                return false;
            };
            let Some(span) = round.checked_sub(seg.vaddr) else {
                crate::log_error!("image", "dynamic Image segment span underflow");
                return false;
            };
            if span % 4096 != 0 {
                crate::log_error!("image", "dynamic Image segment span is not page aligned");
                return false;
            }
            let Some(sum) = pages.checked_add(span / 4096) else {
                crate::log_error!("image", "dynamic Image page-count overflow");
                return false;
            };
            if sum > MAX_LOAD_PAGES {
                crate::log_error!("image", "dynamic Image exceeds load-page budget");
                return false;
            }
            pages = sum;
            top = top.max(end);
            load_base = load_base.min(seg.vaddr & !4095);
        }
        let Some(stack) = top.checked_add(4095).map(|v| v & !4095) else {
            crate::log_error!("image", "dynamic Image stack rounding overflow");
            return false;
        };
        let Some(stack_end) = stack.checked_add(4096) else {
            crate::log_error!("image", "dynamic Image stack-end overflow");
            return false;
        };
        if stack_end > 0x0000_8000_0000_0000 || load_base == u64::MAX {
            crate::log_error!(
                "image",
                "dynamic Image stack or load-base preflight refused"
            );
            return false;
        }
        let e = &mut (*REG.get()).entries[idx];
        e.kind = parsed.kind;
        e.entry = parsed.entry;
        e.load_base = parsed.image_start.min(load_base);
        e.image_end = parsed.image_end;
        e.rela_offset = parsed.rela_offset;
        e.rela_count = parsed.rela_count;
        true
    })
}

/// The executable type recorded by production validation; no randomized
/// per-launch address is stored in this immutable Image metadata.
pub fn kind(id: u32) -> Option<elf::ImageKind> {
    without_interrupts(|| unsafe {
        (*REG.get())
            .entries
            .iter()
            .find(|e| e.state == State::Live && e.id == id)
            .map(|e| e.kind)
    })
}

/// Bounded validation facts retained beside the immutable Image bytes.
pub fn pie_layout(id: u32) -> Option<(u64, u64, u64, usize)> {
    without_interrupts(|| unsafe {
        let e = (*REG.get())
            .entries
            .iter()
            .find(|e| e.state == State::Live && e.id == id)?;
        (e.kind == elf::ImageKind::StaticPie).then_some((
            e.load_base,
            e.image_end,
            e.rela_offset,
            e.rela_count,
        ))
    })
}

/// Metadata from the exact live Image capability. This is descriptive loader
/// state; possessing the Image cap remains the only spawn authority.
pub fn info(id: u32) -> Option<(u64, u64, u64)> {
    without_interrupts(|| unsafe {
        let entry = (*REG.get())
            .entries
            .iter()
            .find(|e| e.state == State::Live && e.id == id)?;
        Some((entry.entry, entry.load_base, entry.len as u64))
    })
}

/// Make an ID LIVE immediately before mint. On a mint refusal `undo_mint`
/// unpublishes it and preserves next_id. Single-core IF=0 throughout.
pub fn begin_mint(idx: usize) -> u32 {
    without_interrupts(|| unsafe {
        let r = &mut *REG.get();
        assert!(r.entries[idx].state == State::Reserved && r.next_id <= u32::MAX as u64);
        let id = r.next_id as u32;
        r.entries[idx].id = id;
        r.entries[idx].state = State::Live;
        id
    })
}
pub fn commit_mint(idx: usize) {
    without_interrupts(|| unsafe {
        let r = &mut *REG.get();
        assert!(
            r.entries[idx].state == State::Live
                && r.entries[idx].refs == 1
                && r.next_id == r.entries[idx].id as u64
        );
        r.next_id += 1;
    })
}
pub fn undo_mint(idx: usize) {
    without_interrupts(|| unsafe {
        let e = &mut (*REG.get()).entries[idx];
        assert!(e.state == State::Live && e.refs == 0 && e.pins == 0);
        e.retire();
    })
}

pub fn add_cap(cap: Cap) {
    if let CapObj::Image { img_id } = cap.obj {
        if img_id < FIRST {
            return;
        }
        without_interrupts(|| unsafe {
            if let Some(e) = (*REG.get())
                .entries
                .iter_mut()
                .find(|e| e.state == State::Live && e.id == img_id)
            {
                e.refs = e.refs.checked_add(1).expect("dynamic Image ref overflow");
            }
            // Revoked/retired copies remain inert and are not counted.
        });
    }
}
pub fn drop_cap(cap: Cap) {
    if let CapObj::Image { img_id } = cap.obj {
        if img_id < FIRST {
            return;
        }
        without_interrupts(|| unsafe {
            if let Some(e) = (*REG.get())
                .entries
                .iter_mut()
                .find(|e| e.state == State::Live && e.id == img_id)
            {
                assert!(e.refs > 0, "dynamic Image reference underflow");
                e.refs -= 1;
                if e.refs == 0 {
                    e.state = State::Revoked;
                    if e.pins == 0 {
                        e.retire();
                    }
                }
            }
        });
    }
}

/// A pin survives caller cap deletion/revoke during loading. The borrowed
/// static bytes cannot be overwritten until unpin on every spawn exit.
pub fn pin(id: u32) -> Option<&'static [u8]> {
    without_interrupts(|| unsafe {
        let e = (*REG.get())
            .entries
            .iter_mut()
            .find(|e| e.state == State::Live && e.id == id)?;
        e.pins = e.pins.checked_add(1).expect("dynamic Image pin overflow");
        Some(core::slice::from_raw_parts(e.bytes.as_ptr(), e.len))
    })
}
pub fn unpin(id: u32) {
    without_interrupts(|| unsafe {
        let e = (*REG.get())
            .entries
            .iter_mut()
            .find(|e| e.id == id && e.state != State::Free)
            .expect("lost dynamic Image pin");
        assert!(e.pins > 0);
        e.pins -= 1;
        if e.state == State::Revoked && e.pins == 0 {
            e.retire();
        }
    });
}
pub fn revoke(id: u32) -> bool {
    without_interrupts(|| unsafe {
        let Some(e) = (*REG.get())
            .entries
            .iter_mut()
            .find(|e| e.state == State::Live && e.id == id)
        else {
            return false;
        };
        e.state = State::Revoked;
        if e.pins == 0 {
            e.retire();
        }
        true
    })
}

/// Independent stable-boundary conservation walk; does NOT consult the
/// production add_cap/drop_cap hooks or infer counts from registry.refs.
/// Queue escrow and process slots each count exactly once by FULL ID.
pub fn assert_conservation() {
    without_interrupts(|| unsafe {
        let r = &*REG.get();
        for e in &r.entries {
            if e.state == State::Reserved {
                crate::halt::halt_machine("Image reservation leaked across stable boundary");
            }
            if e.state == State::Revoked {
                if e.pins == 0 || e.pins != crate::spawn::loader_pin_count(e.id) {
                    crate::halt::halt_machine("retiring Image pin conservation failure");
                }
                continue;
            }
            if e.state != State::Live {
                continue;
            }
            let mut actual = 0u32;
            let mut count = |c: Cap| {
                if c.obj == (CapObj::Image { img_id: e.id }) {
                    actual = actual.checked_add(1).expect("Image oracle count overflow");
                }
            };
            crate::proc::for_each_cap(&mut count);
            crate::ipc::for_each_staged_cap(&mut count);
            if actual != e.refs || e.refs == 0 || e.pins != crate::spawn::loader_pin_count(e.id) {
                crate::halt::halt_machine("Image reference/pin conservation failure");
            }
        }
    });
}

pub fn active() -> bool {
    without_interrupts(|| unsafe { (*REG.get()).entries.iter().any(|e| e.state == State::Live) })
}
