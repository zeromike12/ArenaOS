//! Window policy holds descriptive handles. The service supplies authority
//! from its own exact backing / held Process associations, never caller IDs.
use arena_ui::metrics as m;
/// Concurrent ordinary windows and managed sessions (ADR-0088: measured
/// Phase-12 resource envelope for 32 real application processes).
pub const MAX_WINDOWS: usize = 32;
pub const EVENT_DEPTH: usize = 32;
pub const MIN_WIDTH: u16 = 80;
pub const MIN_HEIGHT: u16 = 60;
/// Largest surface any client may ever declare (the largest supported
/// screen). A session's real bound is the work area of the actual screen.
pub const SURFACE_MAX_WIDTH: u16 = 1024;
pub const SURFACE_MAX_HEIGHT: u16 = 768;
/// Bounded transient (popup) surface: one per window, at most this many
/// pixels, held in the final pages of the session's own reservation.
pub const TRANSIENT_MAX_PIXELS: usize = 65536;
pub const TRANSIENT_MAX_WIDTH: u16 = 512;
pub const TRANSIENT_MAX_HEIGHT: u16 = 384;
pub const TRANSIENT_MIN: u16 = 8;
/// A title-bar press becomes a move only after this much motion.
pub const DRAG_THRESHOLD: i32 = 4;
/// Two primary presses on the same title bar this close in time and
/// space are a double click (maximize / restore).
pub const DOUBLE_CLICK_US: u64 = 400_000;
/// Resize hit zone outside a resizable window's frame (plus two pixels of
/// the frame itself).
pub const RESIZE_ZONE: i32 = 6;
/// Key repeat: first repeat after the delay, then at the interval. A
/// press of the same key from the host (its own autorepeat) restarts it.
pub const REPEAT_DELAY_US: u64 = 500_000;
pub const REPEAT_INTERVAL_US: u64 = 33_000;
/// Modifier bits carried with key events.
pub const MOD_SHIFT: u8 = 1;
pub const MOD_CTRL: u8 = 2;
pub const MOD_ALT: u8 = 4;
pub const MOD_SUPER: u8 = 8;
/// Modifier bits a window's `Pointer` buttons carry above the three button
/// bits (Phase 11.8: Shift range and Ctrl toggle selection by pointer).
pub const POINTER_SHIFT: u8 = 0x40;
pub const POINTER_CTRL: u8 = 0x80;
pub const EDGE_LEFT: u8 = 1;
pub const EDGE_RIGHT: u8 = 2;
pub const EDGE_TOP: u8 = 4;
pub const EDGE_BOTTOM: u8 = 8;
/// Title-bar buttons (window-local, right to left: close, maximize for
/// resizable windows, minimize).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Close = 1,
    Maximize = 2,
    Minimize = 3,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Invalid,
    Full,
    Stale,
    QueueFull,
    Exhausted,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Key(u16),
    Pointer {
        x: i32,
        y: i32,
        buttons: u8,
    },
    Close,
    Focus(bool),
    /// A key pressed while Ctrl, Alt or Super is held (shortcuts).
    Chord {
        code: u16,
        mods: u8,
    },
    /// Scroll wheel notches over the window (positive: away from the user).
    Wheel {
        x: i32,
        y: i32,
        delta: i8,
    },
    /// The window policy gave the window this size; the client should
    /// re-lay out and publish a surface of exactly this size (Resize).
    Configure {
        width: u16,
        height: u16,
    },
    /// Pointer inside (or captured by) the window's transient surface,
    /// in transient-surface coordinates.
    PopupPointer {
        x: i32,
        y: i32,
        buttons: u8,
    },
    /// The window policy dismissed this transient surface (outside press,
    /// focus loss, window move). Its handle is stale from now on.
    Dismissed(u64),
    /// The trusted chooser this client asked for finished (chosen or
    /// cancelled); the client collects the outcome with `TakeGrant`.
    Chosen,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    Launch(usize),
    Close(u64),
    Changed,
}
/// What a transient surface is for; decides dismissal and input policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PopupKind {
    /// Dismissed by any press outside it (the press is consumed) and by
    /// its owner losing focus or moving.
    Menu = 1,
    /// Never takes input; dismissed by any press or focus change.
    Tooltip = 2,
    /// Modal to its owner: presses on the owner outside the dialog are
    /// swallowed; it survives focus changes and moves with its owner.
    Dialog = 3,
}
impl PopupKind {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::Menu),
            2 => Some(Self::Tooltip),
            3 => Some(Self::Dialog),
            _ => None,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Popup {
    pub handle: u64,
    pub kind: PopupKind,
    /// Screen position (kept fully on screen).
    pub x: i32,
    pub y: i32,
    pub width: u16,
    pub height: u16,
}
impl Popup {
    fn contains(self, x: i32, y: i32) -> bool {
        x >= self.x
            && y >= self.y
            && x < self.x + i32::from(self.width)
            && y < self.y + i32::from(self.height)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    pub handle: u64,
    /// Unique surface object key, used only to prevent one backing being
    /// published as multiple windows.
    pub backing: u64,
    /// Authenticated application-session owner for all of its windows.
    pub owner: u64,
    pub x: i32,
    pub y: i32,
    pub width: u16,
    pub height: u16,
    pub z: u64,
    pub minimized: bool,
    /// The client declared it re-lays out to any size >= `min`.
    pub resizable: bool,
    pub min: (u16, u16),
    pub popup: Option<Popup>,
    /// Free geometry to return to from maximized or snapped placement.
    pub restore: Option<(i32, i32, u16, u16)>,
    /// Current placement (only meaningful while `restore` is set).
    pub placement: Placement,
    queue: [Option<Event>; EVENT_DEPTH],
    head: usize,
    len: usize,
}
impl Window {
    fn enqueue(&mut self, e: Event) -> Result<(), Error> {
        // A newer size supersedes an older one still queued: a burst of
        // policy resizes costs one queue slot, never a queue overflow.
        if let Event::Configure { .. } = e
            && self.len > 0
        {
            let last = (self.head + self.len - 1) % EVENT_DEPTH;
            if let Some(Event::Configure { .. }) = self.queue[last] {
                self.queue[last] = Some(e);
                return Ok(());
            }
        }
        if self.len == EVENT_DEPTH {
            return Err(Error::QueueFull);
        }
        self.queue[(self.head + self.len) % EVENT_DEPTH] = Some(e);
        self.len += 1;
        Ok(())
    }
    fn pop(&mut self) -> Option<Event> {
        if self.len == 0 {
            return None;
        }
        let e = self.queue[self.head].take();
        self.head = (self.head + 1) % EVENT_DEPTH;
        self.len -= 1;
        e
    }
    fn contains(self, x: i32, y: i32) -> bool {
        !self.minimized
            && i64::from(x) >= i64::from(self.x)
            && i64::from(y) >= i64::from(self.y)
            && i64::from(x) < i64::from(self.x) + i64::from(self.width)
            && i64::from(y) < i64::from(self.y) + i64::from(self.height)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    Free,
    Maximized,
    Left,
    Right,
}
/// What a pointer press or motion lands on, top-most first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Window(u64),
    Popup(u64),
    /// The resize zone of a resizable window.
    Edge(u64, u8),
}
/// A pointer grab held by the window policy itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Grab {
    /// Pressed on a title bar: becomes a move after `DRAG_THRESHOLD`.
    Move {
        handle: u64,
        dx: i32,
        dy: i32,
        origin: (i32, i32),
        moving: bool,
    },
    /// Dragging a frame edge or corner.
    Resize {
        handle: u64,
        edges: u8,
        start: (i32, i32, u16, u16),
        origin: (i32, i32),
    },
}
pub struct State {
    windows: [Option<Window>; MAX_WINDOWS],
    next: u64,
    z: u64,
    focused: Option<u64>,
    grab: Option<Grab>,
    capture: Option<Target>,
    /// Monotonic time of the current input (set by the broker).
    now: u64,
    /// Last primary press on a title bar: (window, x, y, time).
    last_click: Option<(u64, i32, i32, u64)>,
    /// Modifier state of the latest key event.
    pub mods: u8,
    /// Key and chord events delivered to windows (diagnostics).
    pub delivered: u64,
    /// Key repeat: (code, modifiers, next due time).
    repeat: Option<(u16, u8, u64)>,
    /// Alt+Tab switcher: index of the selected window in `cycle_order`.
    switcher: Option<usize>,
    /// Where a title drag would snap if released now.
    snap: Option<Placement>,
    /// Title-bar button under the pointer (hover feedback).
    pub hover: Option<(u64, Button)>,
    buttons: u8,
    pub pointer: (i32, i32),
    screen: (i32, i32),
    dock_items: usize,
    pub dropped: u64,
}
impl State {
    fn max_title_y(&self) -> i32 {
        (self.screen.1 - m::DOCK_HEIGHT - m::TITLE_HEIGHT).max(m::SYSTEM_BAR_HEIGHT)
    }
    pub const fn new(width: u16, height: u16) -> Result<Self, Error> {
        if width < 320 || height < 240 || width > 1024 || height > 768 {
            return Err(Error::Invalid);
        }
        Ok(Self {
            windows: [None; MAX_WINDOWS],
            next: 1,
            z: 1,
            focused: None,
            grab: None,
            capture: None,
            now: 0,
            last_click: None,
            mods: 0,
            delivered: 0,
            repeat: None,
            switcher: None,
            snap: None,
            hover: None,
            buttons: 0,
            pointer: (0, 0),
            screen: (width as i32, height as i32),
            dropped: 0,
            dock_items: 6,
        })
    }
    pub fn configure_screen(
        &mut self,
        width: u16,
        height: u16,
        dock_items: usize,
    ) -> Result<(), Error> {
        if width < 320
            || height < 240
            || width > 1024
            || height > 768
            || dock_items == 0
            || dock_items > 12
        {
            return Err(Error::Invalid);
        }
        self.screen = (width as i32, height as i32);
        self.dock_items = dock_items;
        Ok(())
    }
    /// Update the number of live dock hit targets without changing the
    /// display mode. An empty dock has no launch hit targets.
    pub fn configure_dock_items(&mut self, dock_items: usize) -> Result<(), Error> {
        if dock_items > 12 {
            return Err(Error::Invalid);
        }
        self.dock_items = dock_items;
        Ok(())
    }
    /// Screen minus the system bar and the dock: where windows live and the
    /// size a maximized window takes.
    pub fn work_area(&self) -> (i32, i32, u16, u16) {
        let h = self.screen.1 - m::SYSTEM_BAR_HEIGHT - m::DOCK_HEIGHT;
        (
            0,
            m::SYSTEM_BAR_HEIGHT,
            self.screen.0 as u16,
            h.max(i32::from(MIN_HEIGHT)) as u16,
        )
    }
    /// Largest surface a window on this screen can have.
    pub fn max_surface(&self) -> (u16, u16) {
        let (_, _, w, h) = self.work_area();
        (w, h)
    }
    pub fn windows(&self) -> impl Iterator<Item = &Window> {
        self.windows.iter().flatten()
    }
    pub fn focused(&self) -> Option<u64> {
        self.focused
    }
    pub fn find(&self, handle: u64) -> Option<&Window> {
        self.windows().find(|w| w.handle == handle)
    }
    pub fn owned(&self, owner: u64, handle: u64) -> bool {
        self.find(handle).is_some_and(|w| w.owner == owner)
    }
    /// The live transient surface `handle` and the window that owns it.
    pub fn find_popup(&self, handle: u64) -> Option<(&Window, Popup)> {
        self.windows()
            .find_map(|w| w.popup.filter(|p| p.handle == handle).map(|p| (w, p)))
    }
    pub fn popup_owned(&self, owner: u64, handle: u64) -> bool {
        self.find_popup(handle)
            .is_some_and(|(w, _)| w.owner == owner)
    }
    fn slot(&self, handle: u64) -> Result<usize, Error> {
        self.windows
            .iter()
            .position(|w| w.is_some_and(|w| w.handle == handle))
            .ok_or(Error::Stale)
    }
    pub fn create(&mut self, backing: u64, width: u16, height: u16) -> Result<u64, Error> {
        self.create_owned(backing, backing, width, height)
    }

    /// Create another independently backed window for one existing
    /// application owner. Surface identity is unique per window; owner
    /// identity is shared by all windows in the same application session.
    pub fn create_owned(
        &mut self,
        owner: u64,
        backing: u64,
        width: u16,
        height: u16,
    ) -> Result<u64, Error> {
        let (max_w, max_h) = self.max_surface();
        if owner == 0
            || backing == 0
            || self.windows().any(|w| w.backing == backing)
            || width < MIN_WIDTH
            || height < MIN_HEIGHT
            || width > max_w
            || height > max_h
        {
            return Err(Error::Invalid);
        }
        let i = self
            .windows
            .iter()
            .position(Option::is_none)
            .ok_or(Error::Full)?;
        let handle = self.next;
        let next = self.next.checked_add(1).ok_or(Error::Exhausted)?;
        let next_z = self.z.checked_add(1).ok_or(Error::Exhausted)?;
        self.z.checked_add(2).ok_or(Error::Exhausted)?;
        // Capacity and counter preflight before focus, queues or surfaces mutate.
        // The first six slots keep the historical cascade; later slots start
        // a second cascade offset to the right.
        let lane = (i % 6) as i32;
        let column = (i / 6) as i32 * m::CASCADE_X * 7;
        self.windows[i] = Some(Window {
            handle,
            backing,
            owner,
            x: (m::WINDOW_START_X + column + lane * m::CASCADE_X)
                .min(self.screen.0 - m::VISIBLE_TITLE_WIDTH),
            y: (m::WINDOW_START_Y + lane * m::CASCADE_Y)
                .clamp(m::SYSTEM_BAR_HEIGHT, self.max_title_y()),
            width,
            height,
            z: self.z,
            minimized: false,
            resizable: false,
            min: (width, height),
            popup: None,
            restore: None,
            placement: Placement::Free,
            queue: [None; EVENT_DEPTH],
            head: 0,
            len: 0,
        });
        self.next = next;
        self.z = next_z;
        self.activate(handle)?;
        Ok(handle)
    }
    pub fn retire(&mut self, handle: u64) -> Result<(), Error> {
        let i = self.slot(handle)?;
        let popup = self.windows[i].and_then(|w| w.popup).map(|p| p.handle);
        self.windows[i] = None;
        if self.grab.is_some_and(|g| grab_handle(g) == handle) {
            self.grab = None;
            self.snap = None;
        }
        if self.hover.is_some_and(|(h, _)| h == handle) {
            self.hover = None;
        }
        if self.last_click.is_some_and(|c| c.0 == handle) {
            self.last_click = None;
        }
        if self.capture == Some(Target::Window(handle))
            || popup.is_some_and(|p| self.capture == Some(Target::Popup(p)))
        {
            self.capture = None;
        }
        if self.focused == Some(handle) {
            self.focused = None;
            let next = self
                .windows()
                .filter(|w| !w.minimized)
                .max_by_key(|w| w.z)
                .map(|w| w.handle);
            if let Some(h) = next {
                self.focused = Some(h);
                self.send(h, Event::Focus(true));
            }
        }
        Ok(())
    }
    pub fn activate(&mut self, handle: u64) -> Result<(), Error> {
        let i = self.slot(handle)?;
        let z = self.z;
        let next = self.z.checked_add(1).ok_or(Error::Exhausted)?;
        if let Some(old) = self.focused.filter(|old| *old != handle) {
            self.send(old, Event::Focus(false));
            self.dismiss_transient(old, false);
        }
        let w = self.windows[i].as_mut().ok_or(Error::Stale)?;
        w.z = z;
        w.minimized = false;
        self.z = next;
        self.focused = Some(handle);
        self.send(handle, Event::Focus(true));
        Ok(())
    }
    pub fn send(&mut self, handle: u64, e: Event) -> bool {
        if let Ok(i) = self.slot(handle) {
            if self.windows[i]
                .as_mut()
                .is_some_and(|w| w.enqueue(e).is_ok())
            {
                return true;
            }
            self.dropped = self.dropped.saturating_add(1);
        }
        false
    }
    /// Events are queued for `handle` and not yet polled. A wake hint for
    /// the broker only; it grants nothing.
    pub fn pending(&self, handle: u64) -> bool {
        self.find(handle).is_some_and(|w| w.len > 0)
    }
    pub fn poll(&mut self, handle: u64) -> Result<Option<Event>, Error> {
        let i = self.slot(handle)?;
        Ok(self.windows[i].as_mut().ok_or(Error::Stale)?.pop())
    }
    /// The client of `handle` declares that it re-lays out to any size of
    /// at least `min_w` x `min_h` (and at most the work area).
    pub fn set_resizable(&mut self, handle: u64, min_w: u16, min_h: u16) -> Result<(), Error> {
        let (max_w, max_h) = self.max_surface();
        if min_w < MIN_WIDTH || min_h < MIN_HEIGHT || min_w > max_w || min_h > max_h {
            return Err(Error::Invalid);
        }
        let i = self.slot(handle)?;
        let w = self.windows[i].as_mut().ok_or(Error::Stale)?;
        if w.width < min_w || w.height < min_h {
            return Err(Error::Invalid);
        }
        w.resizable = true;
        w.min = (min_w, min_h);
        Ok(())
    }
    /// Policy resize: clamp to the window's declared minimum and the work
    /// area, then tell its client (coalesced Configure). Returns whether
    /// the size changed. Fixed-size windows refuse.
    pub fn resize(&mut self, handle: u64, width: u16, height: u16) -> Result<bool, Error> {
        let (max_w, max_h) = self.max_surface();
        let i = self.slot(handle)?;
        let w = self.windows[i].as_mut().ok_or(Error::Stale)?;
        if !w.resizable {
            return Err(Error::Invalid);
        }
        let size = (width.clamp(w.min.0, max_w), height.clamp(w.min.1, max_h));
        if size == (w.width, w.height) {
            return Ok(false);
        }
        w.width = size.0;
        w.height = size.1;
        if w.popup.is_some_and(|p| p.kind != PopupKind::Dialog) {
            self.dismiss_transient(handle, false);
        }
        self.send(
            handle,
            Event::Configure {
                width: size.0,
                height: size.1,
            },
        );
        Ok(true)
    }
    /// Open the transient surface of window `owner` at window-local
    /// (`x`, `y`). Only the focused window may open one (no focus
    /// stealing); a second open replaces the first, whose handle becomes
    /// stale. The surface is kept fully on screen.
    #[allow(clippy::too_many_arguments)]
    pub fn open_popup(
        &mut self,
        backing: u64,
        owner: u64,
        kind: PopupKind,
        x: i32,
        y: i32,
        width: u16,
        height: u16,
    ) -> Result<u64, Error> {
        if !self.owned(backing, owner)
            || self.focused != Some(owner)
            || width < TRANSIENT_MIN
            || height < TRANSIENT_MIN
            || width > TRANSIENT_MAX_WIDTH
            || height > TRANSIENT_MAX_HEIGHT
            || usize::from(width) * usize::from(height) > TRANSIENT_MAX_PIXELS
            || i32::from(width) > self.screen.0
            || i32::from(height) > self.screen.1 - m::SYSTEM_BAR_HEIGHT
        {
            return Err(Error::Invalid);
        }
        let handle = self.next;
        let next = self.next.checked_add(1).ok_or(Error::Exhausted)?;
        let i = self.slot(owner)?;
        let screen = self.screen;
        let w = self.windows[i].ok_or(Error::Stale)?;
        if w.minimized
            || !(-i32::from(SURFACE_MAX_WIDTH)..=i32::from(SURFACE_MAX_WIDTH)).contains(&x)
            || !(-i32::from(SURFACE_MAX_HEIGHT)..=i32::from(SURFACE_MAX_HEIGHT)).contains(&y)
        {
            return Err(Error::Invalid);
        }
        let px = (w.x + x).clamp(0, screen.0 - i32::from(width));
        let py = (w.y + y).clamp(m::SYSTEM_BAR_HEIGHT, screen.1 - i32::from(height));
        if let Some(old) = w.popup
            && self.capture == Some(Target::Popup(old.handle))
        {
            self.capture = None;
        }
        let w = self.windows[i].as_mut().ok_or(Error::Stale)?;
        w.popup = Some(Popup {
            handle,
            kind,
            x: px,
            y: py,
            width,
            height,
        });
        self.next = next;
        Ok(handle)
    }
    /// The owning client closes its own transient surface.
    pub fn close_popup(&mut self, backing: u64, handle: u64) -> Result<(), Error> {
        if !self.popup_owned(backing, handle) {
            return Err(Error::Stale);
        }
        let owner = self
            .find_popup(handle)
            .map(|(w, _)| w.handle)
            .ok_or(Error::Stale)?;
        let i = self.slot(owner)?;
        if let Some(w) = self.windows[i].as_mut() {
            w.popup = None;
        }
        if self.capture == Some(Target::Popup(handle)) {
            self.capture = None;
        }
        Ok(())
    }
    /// Policy dismissal of `owner`'s transient surface (dialogs only when
    /// `dialogs`); the client learns its handle went stale.
    fn dismiss_transient(&mut self, owner: u64, dialogs: bool) -> bool {
        let Ok(i) = self.slot(owner) else {
            return false;
        };
        let Some(p) = self.windows[i].and_then(|w| w.popup) else {
            return false;
        };
        if p.kind == PopupKind::Dialog && !dialogs {
            return false;
        }
        if let Some(w) = self.windows[i].as_mut() {
            w.popup = None;
        }
        if self.capture == Some(Target::Popup(p.handle)) {
            self.capture = None;
        }
        self.send(owner, Event::Dismissed(p.handle));
        true
    }
    /// Monotonic time of the input about to be processed (double click,
    /// key repeat). Set by the broker before `pointer`/`key_input`.
    pub fn set_time(&mut self, now: u64) {
        self.now = now;
    }
    pub fn key(&mut self, key: u16) -> bool {
        self.focused.is_some_and(|h| self.send(h, Event::Key(key)))
    }
    /// Deliver a key to the focused window: plain (and shifted) keys as
    /// `Key`, keys under Ctrl/Alt/Super and shifted navigation keys
    /// (arrows, Home, End, Delete: codes from 256) as `Chord`.
    fn deliver(&mut self, code: u16, mods: u8) -> bool {
        let Some(h) = self.focused else {
            return false;
        };
        self.delivered = self.delivered.wrapping_add(1);
        let shifted_navigation = mods & MOD_SHIFT != 0 && code >= 256;
        if mods & (MOD_CTRL | MOD_ALT | MOD_SUPER) == 0 && !shifted_navigation {
            self.send(h, Event::Key(code))
        } else {
            self.send(h, Event::Chord { code, mods })
        }
    }
    /// Whether (`x`, `y`) is bare desktop: no window, popup or edge there,
    /// below the system bar and above the dock, with no drag in progress.
    pub fn bare(&self, x: i32, y: i32) -> bool {
        self.grab.is_none()
            && self.capture.is_none()
            && self.target(x, y).is_none()
            && y >= m::SYSTEM_BAR_HEIGHT
            && y < self.screen.1 - m::DOCK_HEIGHT
    }
    /// No window has keyboard focus (a click on the bare desktop).
    pub fn blur(&mut self) {
        if let Some(h) = self.focused.take() {
            self.send(h, Event::Focus(false));
            self.dismiss_transient(h, false);
        }
    }
    fn pointer_mods(&self) -> u8 {
        (if self.mods & MOD_SHIFT != 0 {
            POINTER_SHIFT
        } else {
            0
        }) | (if self.mods & MOD_CTRL != 0 {
            POINTER_CTRL
        } else {
            0
        })
    }
    /// Historical entry point: a press without modifiers.
    pub fn keyboard_action(&mut self, key: u16) -> Action {
        let mods = self.mods;
        self.key_input(key, true, mods)
    }
    /// One key event from the input producer. `MODIFIERS` reports a
    /// modifier change only. Window-management chords are policy here;
    /// everything else goes to the focused window, with key repeat.
    pub fn key_input(&mut self, code: u16, pressed: bool, mods: u8) -> Action {
        use crate::input_wire as input;
        self.mods = mods & 0x0f;
        if code == input::MODIFIERS {
            // Releasing Alt commits the switcher selection.
            if self.mods & MOD_ALT == 0 && self.switcher.is_some() {
                return self.commit_switcher();
            }
            return Action::None;
        }
        if !pressed {
            if self.repeat.is_some_and(|(c, _, _)| c == code) {
                self.repeat = None;
            }
            return Action::None;
        }
        let alt = self.mods & MOD_ALT != 0;
        let sup = self.mods & MOD_SUPER != 0;
        let shift = self.mods & MOD_SHIFT != 0;
        match code {
            input::LAUNCH_FIRST..=input::LAUNCH_LAST => {
                return Action::Launch((code - input::LAUNCH_FIRST) as usize);
            }
            input::CLOSE_FOCUSED => {
                return self.focused.map(Action::Close).unwrap_or(Action::None);
            }
            input::FOCUS_NEXT => {
                let mut order = [(0u64, 0u64); MAX_WINDOWS];
                let mut count = 0;
                for w in self.windows() {
                    order[count] = (w.z, w.handle);
                    count += 1;
                }
                order[..count].sort_unstable();
                return if count > 0 && self.activate(order[0].1).is_ok() {
                    Action::Changed
                } else {
                    Action::None
                };
            }
            9 if alt => return self.advance_switcher(shift),
            27 if self.switcher.is_some() => {
                self.switcher = None;
                return Action::Changed;
            }
            256..=259 if sup => {
                let Some(h) = self.focused else {
                    return Action::None;
                };
                let done = match code {
                    256 => self.snap_to(h, Placement::Left),
                    257 => self.snap_to(h, Placement::Right),
                    258 => self.maximize(h),
                    _ => {
                        if self.find(h).is_some_and(|w| w.restore.is_some()) {
                            self.restore(h)
                        } else {
                            self.minimize(h)
                        }
                    }
                };
                return if done.is_ok() {
                    Action::Changed
                } else {
                    Action::None
                };
            }
            _ => {}
        }
        if self.switcher.is_some() {
            return Action::None;
        }
        let delivered = self.deliver(code, self.mods);
        // A host autorepeat press of the held key restarts the delay, so
        // host and broker repeat never stack.
        self.repeat = delivered.then_some((code, self.mods, self.now + REPEAT_DELAY_US));
        Action::None
    }
    /// When the held key repeats next, if one is held.
    pub fn repeat_deadline(&self) -> Option<u64> {
        self.repeat.map(|(_, _, due)| due)
    }
    /// Deliver a due repeat (call at or after `repeat_deadline`).
    pub fn repeat_tick(&mut self, now: u64) -> bool {
        match self.repeat {
            Some((code, mods, due)) if due <= now => {
                if self.deliver(code, mods) {
                    self.repeat = Some((code, mods, now + REPEAT_INTERVAL_US));
                    true
                } else {
                    self.repeat = None;
                    false
                }
            }
            _ => false,
        }
    }
    /// Windows most recently used first (the switcher's order).
    pub fn cycle_order(&self) -> ([u64; MAX_WINDOWS], usize) {
        let mut order = [(0u64, 0u64); MAX_WINDOWS];
        let mut n = 0;
        for w in self.windows() {
            order[n] = (u64::MAX - w.z, w.handle);
            n += 1;
        }
        order[..n].sort_unstable();
        let mut out = [0u64; MAX_WINDOWS];
        for (i, (_, h)) in order[..n].iter().enumerate() {
            out[i] = *h;
        }
        (out, n)
    }
    /// The switcher overlay, if open: (selected index, MRU order, count).
    pub fn switcher(&self) -> Option<(usize, [u64; MAX_WINDOWS], usize)> {
        let (order, n) = self.cycle_order();
        self.switcher
            .filter(|_| n > 0)
            .map(|s| (s.min(n - 1), order, n))
    }
    fn advance_switcher(&mut self, back: bool) -> Action {
        let (_, n) = self.cycle_order();
        if n == 0 {
            return Action::None;
        }
        let next = match self.switcher {
            None if back => n - 1,
            None => 1 % n,
            Some(s) if back => (s + n - 1) % n,
            Some(s) => (s + 1) % n,
        };
        self.switcher = Some(next);
        Action::Changed
    }
    fn commit_switcher(&mut self) -> Action {
        let Some((selected, order, _)) = self.switcher() else {
            self.switcher = None;
            return Action::Changed;
        };
        self.switcher = None;
        let _ = self.activate(order[selected]);
        Action::Changed
    }
    fn focus_top(&mut self) {
        let next = self
            .windows()
            .filter(|w| !w.minimized)
            .max_by_key(|w| w.z)
            .map(|w| w.handle);
        self.focused = None;
        if let Some(h) = next {
            self.focused = Some(h);
            self.send(h, Event::Focus(true));
        }
    }
    /// Hide `handle` until restored from the dock or the switcher.
    pub fn minimize(&mut self, handle: u64) -> Result<(), Error> {
        let i = self.slot(handle)?;
        self.dismiss_transient(handle, false);
        let w = self.windows[i].as_mut().ok_or(Error::Stale)?;
        w.minimized = true;
        if self.focused == Some(handle) {
            self.send(handle, Event::Focus(false));
            self.focus_top();
        }
        if self.grab.is_some_and(|g| grab_handle(g) == handle) {
            self.grab = None;
        }
        if self.capture == Some(Target::Window(handle)) {
            self.capture = None;
        }
        Ok(())
    }
    /// Most recently used minimized window whose backing satisfies `pick`.
    pub fn minimized_window(&self, pick: impl Fn(u64) -> bool) -> Option<u64> {
        self.windows()
            .filter(|w| w.minimized && pick(w.owner))
            .max_by_key(|w| w.z)
            .map(|w| w.handle)
    }
    fn place(&mut self, handle: u64, x: i32, y: i32, width: u16, height: u16) -> Result<(), Error> {
        let i = self.slot(handle)?;
        let w = self.windows[i].as_mut().ok_or(Error::Stale)?;
        w.x = x;
        w.y = y;
        self.resize(handle, width, height)?;
        Ok(())
    }
    /// Fill the work area (resizable windows only).
    pub fn maximize(&mut self, handle: u64) -> Result<(), Error> {
        self.place_at(handle, Placement::Maximized)
    }
    /// Left or right half of the work area.
    pub fn snap_to(&mut self, handle: u64, side: Placement) -> Result<(), Error> {
        self.place_at(handle, side)
    }
    fn place_at(&mut self, handle: u64, placement: Placement) -> Result<(), Error> {
        let w = *self.find(handle).ok_or(Error::Stale)?;
        let (wx, wy, ww, wh) = self.work_area();
        // A half never goes below the window's declared minimum (on a
        // narrow screen the two halves then overlap).
        let half = (ww / 2).max(w.min.0).min(ww);
        let (x, width) = match placement {
            Placement::Maximized => (wx, ww),
            Placement::Left => (wx, half),
            Placement::Right => (wx + i32::from(ww - half), half),
            Placement::Free => return self.restore(handle),
        };
        if !w.resizable || w.min.0 > width || w.min.1 > wh {
            return Err(Error::Invalid);
        }
        let saved = w.restore.unwrap_or((w.x, w.y, w.width, w.height));
        self.place(handle, x, wy, width, wh)?;
        let i = self.slot(handle)?;
        if let Some(w) = self.windows[i].as_mut() {
            w.restore = Some(saved);
            w.placement = placement;
            w.minimized = false;
        }
        Ok(())
    }
    /// Back to the free geometry saved before maximize or snap.
    pub fn restore(&mut self, handle: u64) -> Result<(), Error> {
        let w = *self.find(handle).ok_or(Error::Stale)?;
        let Some((x, y, width, height)) = w.restore else {
            return Err(Error::Invalid);
        };
        self.place(handle, x, y, width, height)?;
        let i = self.slot(handle)?;
        if let Some(w) = self.windows[i].as_mut() {
            w.restore = None;
            w.placement = Placement::Free;
        }
        Ok(())
    }
    pub fn toggle_maximize(&mut self, handle: u64) -> Result<(), Error> {
        if self
            .find(handle)
            .is_some_and(|w| w.placement == Placement::Maximized && w.restore.is_some())
        {
            self.restore(handle)
        } else {
            self.maximize(handle)
        }
    }
    /// Where a title drag would snap if released now (preview outline).
    pub fn snap_preview(&self) -> Option<(i32, i32, u16, u16)> {
        let (wx, wy, ww, wh) = self.work_area();
        let min = match self.grab {
            Some(Grab::Move { handle, .. }) => self.find(handle).map_or(0, |w| w.min.0),
            _ => 0,
        };
        let half = (ww / 2).max(min).min(ww);
        match self.snap? {
            Placement::Maximized => Some((wx, wy, ww, wh)),
            Placement::Left => Some((wx, wy, half, wh)),
            Placement::Right => Some((wx + i32::from(ww - half), wy, half, wh)),
            Placement::Free => None,
        }
    }
    /// Title-bar button of `w` at screen (`x`, `y`), if any.
    pub fn title_button(w: &Window, x: i32, y: i32) -> Option<Button> {
        if y < w.y || y >= w.y + m::TITLE_HEIGHT || !w.contains(x, y) {
            return None;
        }
        let right = w.x + i32::from(w.width);
        if x >= right - m::CLOSE_WIDTH {
            return Some(Button::Close);
        }
        // Same placement rule as the chrome that draws the wells.
        use arena_ui::components::control_fits;
        let mut edge = right - m::CLOSE_WIDTH;
        if w.resizable && control_fits(w.x, edge) {
            if x >= edge - m::CLOSE_WIDTH {
                return Some(Button::Maximize);
            }
            edge -= m::CLOSE_WIDTH;
        }
        (control_fits(w.x, edge) && x >= edge - m::CLOSE_WIDTH).then_some(Button::Minimize)
    }
    /// Resize edges of `w` under screen (`x`, `y`): the zone just outside
    /// a free resizable window's frame and its outermost two pixels.
    fn edges(&self, w: &Window, x: i32, y: i32) -> u8 {
        if !w.resizable || w.minimized || w.restore.is_some() {
            return 0;
        }
        let (x0, y0) = (w.x, w.y);
        let (x1, y1) = (w.x + i32::from(w.width), w.y + i32::from(w.height));
        if x < x0 - RESIZE_ZONE
            || x >= x1 + RESIZE_ZONE
            || y < y0 - RESIZE_ZONE
            || y >= y1 + RESIZE_ZONE
        {
            return 0;
        }
        let mut e = 0;
        if x < x0 + 2 {
            e |= EDGE_LEFT;
        }
        if x >= x1 - 2 {
            e |= EDGE_RIGHT;
        }
        if y < y0 + 2 {
            e |= EDGE_TOP;
        }
        if y >= y1 - 2 {
            e |= EDGE_BOTTOM;
        }
        // The top edge never covers the title buttons' strip from inside.
        if e == EDGE_TOP && y >= y0 {
            return 0;
        }
        e
    }
    pub fn hit(&self, x: i32, y: i32) -> Option<u64> {
        self.windows()
            .filter(|w| w.contains(x, y))
            .max_by_key(|w| w.z)
            .map(|w| w.handle)
    }
    /// Top-most thing under the pointer. A window's transient surface sits
    /// directly above its owner, so windows above the owner cover both.
    /// Tooltips never take input.
    fn target(&self, x: i32, y: i32) -> Option<Target> {
        let mut best: Option<(u64, Target)> = None;
        for w in self.windows().filter(|w| !w.minimized) {
            let popup = w
                .popup
                .filter(|p| p.kind != PopupKind::Tooltip && p.contains(x, y));
            let edges = self.edges(w, x, y);
            let hit = match popup {
                Some(p) => Some(Target::Popup(p.handle)),
                None if edges != 0 => Some(Target::Edge(w.handle, edges)),
                None if w.contains(x, y) => Some(Target::Window(w.handle)),
                None => None,
            };
            if let Some(t) = hit
                && best.is_none_or(|(z, _)| w.z > z)
            {
                best = Some((w.z, t));
            }
        }
        best.map(|(_, t)| t)
    }
    fn send_popup_pointer(&mut self, popup: u64, x: i32, y: i32, buttons: u8) {
        if let Some((w, p)) = self.find_popup(popup) {
            let owner = w.handle;
            self.send(
                owner,
                Event::PopupPointer {
                    x: (x - p.x).clamp(-i32::from(p.width), i32::from(p.width)),
                    y: (y - p.y).clamp(-i32::from(p.height), i32::from(p.height)),
                    buttons,
                },
            );
        }
    }
    /// Scroll wheel notches at the pointer: to the window under it.
    pub fn wheel(&mut self, delta: i8) -> Action {
        let (x, y) = self.pointer;
        if delta == 0 {
            return Action::None;
        }
        if let Some(Target::Window(h)) = self.target(x, y) {
            let w = *self.find(h).expect("target window");
            if w.popup.is_some_and(|p| p.kind == PopupKind::Dialog) {
                return Action::None;
            }
            self.send(
                h,
                Event::Wheel {
                    x: x - w.x,
                    y: y - w.y,
                    delta,
                },
            );
        }
        Action::Changed
    }
    /// Continue the window policy's own grab (move or resize).
    fn drive_grab(&mut self, g: Grab, x: i32, y: i32) {
        match g {
            Grab::Move {
                handle,
                mut dx,
                dy,
                origin,
                moving,
            } => {
                if !moving
                    && (x - origin.0).abs() < DRAG_THRESHOLD
                    && (y - origin.1).abs() < DRAG_THRESHOLD
                {
                    return;
                }
                if !moving {
                    // Dragging a maximized or snapped window out restores
                    // its free size under the pointer.
                    if let Some(w) = self.find(handle).copied()
                        && let Some((_, _, rw, _)) = w.restore
                    {
                        let _ = self.restore(handle);
                        dx = dx.min(i32::from(rw) - m::CLOSE_WIDTH * 3).max(8);
                    }
                    self.dismiss_transient(handle, false);
                }
                self.grab = Some(Grab::Move {
                    handle,
                    dx,
                    dy,
                    origin,
                    moving: true,
                });
                let max_y = self.max_title_y();
                let screen = self.screen;
                let resizable = self.find(handle).is_some_and(|w| w.resizable);
                self.snap = if !resizable {
                    None
                } else if x <= 0 {
                    Some(Placement::Left)
                } else if x >= screen.0 - 1 {
                    Some(Placement::Right)
                } else if y <= m::SYSTEM_BAR_HEIGHT {
                    Some(Placement::Maximized)
                } else {
                    None
                };
                if let Ok(i) = self.slot(handle) {
                    let w = self.windows[i].as_mut().expect("checked slot");
                    let old = (w.x, w.y);
                    w.x = (x - dx).clamp(
                        m::VISIBLE_TITLE_WIDTH - w.width as i32,
                        screen.0 - m::VISIBLE_TITLE_WIDTH,
                    );
                    w.y = (y - dy).clamp(m::SYSTEM_BAR_HEIGHT, max_y);
                    // A dialog travels with its owner (kept on screen).
                    let (mx, my) = (w.x - old.0, w.y - old.1);
                    if let Some(p) = w.popup.as_mut() {
                        p.x = (p.x + mx).clamp(0, screen.0 - i32::from(p.width));
                        p.y =
                            (p.y + my).clamp(m::SYSTEM_BAR_HEIGHT, screen.1 - i32::from(p.height));
                    }
                }
            }
            Grab::Resize {
                handle,
                edges,
                start,
                origin,
            } => {
                let Some(w) = self.find(handle).copied() else {
                    return;
                };
                let (max_w, max_h) = self.max_surface();
                let (dx, dy) = (x - origin.0, y - origin.1);
                let (sx, sy, sw, sh) = (start.0, start.1, i32::from(start.2), i32::from(start.3));
                let mut width = sw;
                let mut height = sh;
                if edges & EDGE_RIGHT != 0 {
                    width = sw + dx;
                }
                if edges & EDGE_LEFT != 0 {
                    width = sw - dx;
                }
                if edges & EDGE_BOTTOM != 0 {
                    height = sh + dy;
                }
                if edges & EDGE_TOP != 0 {
                    height = sh - dy;
                }
                let width = width.clamp(i32::from(w.min.0), i32::from(max_w));
                let mut height = height.clamp(i32::from(w.min.1), i32::from(max_h));
                let nx = if edges & EDGE_LEFT != 0 {
                    sx + sw - width
                } else {
                    sx
                };
                let mut ny = if edges & EDGE_TOP != 0 {
                    sy + sh - height
                } else {
                    sy
                };
                if ny < m::SYSTEM_BAR_HEIGHT {
                    height -= m::SYSTEM_BAR_HEIGHT - ny;
                    ny = m::SYSTEM_BAR_HEIGHT;
                }
                let _ = self.place(
                    handle,
                    nx,
                    ny,
                    width as u16,
                    height.max(i32::from(w.min.1)) as u16,
                );
            }
        }
    }
    pub fn pointer(&mut self, x: i32, y: i32, buttons: u8) -> Action {
        let (x, y) = (x.clamp(0, self.screen.0 - 1), y.clamp(0, self.screen.1 - 1));
        let pressed = buttons & 1 != 0 && self.buttons & 1 == 0;
        let released = buttons & 1 == 0 && self.buttons & 1 != 0;
        // Secondary (context) press: activates and reaches the window like a
        // primary press, but never drags, closes or captures.
        let secondary = buttons & 2 != 0 && self.buttons & 2 == 0;
        self.pointer = (x, y);
        self.buttons = buttons & 7;
        if let Some(g) = self.grab {
            if released {
                self.grab = None;
                if let (Grab::Move { handle, .. }, Some(side)) = (g, self.snap.take()) {
                    let _ = self.snap_to(handle, side);
                }
                return Action::Changed;
            }
            if self.find(grab_handle(g)).is_some() {
                self.drive_grab(g, x, y);
                return Action::Changed;
            }
            self.grab = None;
            self.snap = None;
        }
        match self.capture {
            Some(Target::Window(h)) => {
                if let Some(w) = self.find(h).copied() {
                    self.send(
                        h,
                        Event::Pointer {
                            x: (x - w.x).clamp(-(w.width as i32), w.width as i32),
                            y: (y - w.y).clamp(-(w.height as i32), w.height as i32),
                            buttons: (buttons & 7) | self.pointer_mods(),
                        },
                    );
                }
                if released {
                    self.capture = None;
                }
                return Action::Changed;
            }
            Some(Target::Popup(p)) => {
                self.send_popup_pointer(p, x, y, buttons & 7);
                if released {
                    self.capture = None;
                }
                return Action::Changed;
            }
            _ => {}
        }
        let target = self.target(x, y);
        self.hover = match target {
            Some(Target::Window(h)) => self
                .find(h)
                .and_then(|w| Self::title_button(w, x, y))
                .map(|b| (h, b)),
            _ => None,
        };
        if pressed || secondary {
            // Any press dismisses tooltips; a press outside a menu dismisses
            // it and is consumed (it only closes the menu).
            let mut consumed = false;
            let mut owners = [0u64; MAX_WINDOWS];
            let mut n = 0;
            for w in self.windows() {
                if let Some(p) = w.popup {
                    let inside = target == Some(Target::Popup(p.handle));
                    if p.kind == PopupKind::Tooltip || (p.kind == PopupKind::Menu && !inside) {
                        owners[n] = w.handle;
                        n += 1;
                        consumed |= p.kind == PopupKind::Menu;
                    }
                }
            }
            for owner in &owners[..n] {
                self.dismiss_transient(*owner, false);
            }
            if consumed {
                return Action::Changed;
            }
        }
        if secondary
            && !pressed
            && let Some(Target::Window(h)) = target
        {
            let modal = self
                .find(h)
                .is_some_and(|w| w.popup.is_some_and(|p| p.kind == PopupKind::Dialog));
            if self.focused != Some(h) && self.activate(h).is_err() {
                return Action::None;
            }
            if modal {
                return Action::Changed;
            }
        }
        let dock_w = self.dock_items as i32 * m::DOCK_ITEM_WIDTH;
        let dock_x = (self.screen.0 - dock_w) / 2;
        if pressed
            && !matches!(target, Some(Target::Popup(_)))
            && y >= self.screen.1 - m::DOCK_HEIGHT
            && x >= dock_x
            && x < dock_x + dock_w
        {
            return Action::Launch(((x - dock_x) / m::DOCK_ITEM_WIDTH) as usize);
        }
        match target {
            Some(Target::Edge(h, edges)) => {
                if pressed {
                    if self.activate(h).is_err() {
                        return Action::None;
                    }
                    let w = *self.find(h).expect("activated window");
                    self.dismiss_transient(h, false);
                    self.grab = Some(Grab::Resize {
                        handle: h,
                        edges,
                        start: (w.x, w.y, w.width, w.height),
                        origin: (x, y),
                    });
                }
            }
            Some(Target::Popup(p)) => {
                if pressed {
                    let owner = self.find_popup(p).map(|(w, _)| w.handle);
                    if let Some(owner) = owner
                        && self.focused != Some(owner)
                        && self.activate(owner).is_err()
                    {
                        return Action::None;
                    }
                    self.capture = Some(Target::Popup(p));
                }
                self.send_popup_pointer(p, x, y, buttons & 7);
            }
            Some(Target::Window(h)) => {
                if pressed {
                    if self.activate(h).is_err() {
                        return Action::None;
                    }
                    let w = *self.find(h).expect("activated window");
                    if y < w.y + m::TITLE_HEIGHT {
                        match Self::title_button(&w, x, y) {
                            Some(Button::Close) => return Action::Close(h),
                            Some(Button::Maximize) => {
                                let _ = self.toggle_maximize(h);
                                return Action::Changed;
                            }
                            Some(Button::Minimize) => {
                                let _ = self.minimize(h);
                                return Action::Changed;
                            }
                            None => {}
                        }
                        let double = self.last_click.is_some_and(|(lh, lx, ly, t)| {
                            lh == h
                                && self.now.saturating_sub(t) <= DOUBLE_CLICK_US
                                && (x - lx).abs() <= DRAG_THRESHOLD
                                && (y - ly).abs() <= DRAG_THRESHOLD
                        });
                        if double && w.resizable {
                            self.last_click = None;
                            let _ = self.toggle_maximize(h);
                            return Action::Changed;
                        }
                        self.last_click = Some((h, x, y, self.now));
                        self.grab = Some(Grab::Move {
                            handle: h,
                            dx: x - w.x,
                            dy: y - w.y,
                            origin: (x, y),
                            moving: false,
                        });
                        return Action::Changed;
                    }
                    // A modal dialog swallows presses on its owner's body.
                    if w.popup.is_some_and(|p| p.kind == PopupKind::Dialog) {
                        return Action::Changed;
                    }
                    self.capture = Some(Target::Window(h));
                }
                let w = *self.find(h).expect("hit window");
                if w.popup.is_some_and(|p| p.kind == PopupKind::Dialog) {
                    return Action::Changed;
                }
                self.send(
                    h,
                    Event::Pointer {
                        x: x - w.x,
                        y: y - w.y,
                        buttons: (buttons & 7) | self.pointer_mods(),
                    },
                );
            }
            None => {}
        }
        Action::Changed
    }
}
fn grab_handle(g: Grab) -> u64 {
    match g {
        Grab::Move { handle, .. } | Grab::Resize { handle, .. } => handle,
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_owner_has_multiple_independently_stateful_windows() {
        let mut state = State::new(800, 600).unwrap();
        let first = state.create_owned(9, 101, 160, 120).unwrap();
        let second = state.create_owned(9, 102, 180, 140).unwrap();
        let third = state.create_owned(9, 103, 200, 160).unwrap();
        assert!(state.owned(9, first));
        assert!(state.owned(9, second));
        assert!(state.owned(9, third));
        assert!(!state.owned(10, second));
        assert_ne!(
            state.find(first).unwrap().backing,
            state.find(second).unwrap().backing
        );
        assert_eq!(
            (
                state.find(second).unwrap().width,
                state.find(second).unwrap().height
            ),
            (180, 140)
        );
        state.minimize(second).unwrap();
        assert!(state.find(first).is_some_and(|window| !window.minimized));
        assert!(state.find(second).is_some_and(|window| window.minimized));
        assert_eq!(state.minimized_window(|owner| owner == 9), Some(second));
        state.retire(second).unwrap();
        assert!(state.find(first).is_some() && state.find(third).is_some());
    }
    #[test]
    fn desktop_keyboard_launch_cycle_close_preserves_ordinary_key_focus() {
        let mut s = State::new(800, 600).unwrap();
        assert_eq!(
            s.keyboard_action(crate::input_wire::LAUNCH_FIRST),
            Action::Launch(0)
        );
        let a = s.create(1, 100, 100).unwrap();
        let b = s.create(2, 100, 100).unwrap();
        assert_eq!(s.focused(), Some(b));
        assert_eq!(
            s.keyboard_action(crate::input_wire::FOCUS_NEXT),
            Action::Changed
        );
        assert_eq!(s.focused(), Some(a));
        assert_eq!(
            s.keyboard_action(crate::input_wire::CLOSE_FOCUSED),
            Action::Close(a)
        );
        assert_eq!(s.keyboard_action(116), Action::None);
        while let Some(e) = s.poll(a).unwrap() {
            if e == Event::Key(116) {
                return;
            }
        }
        panic!("ordinary key did not reach focused owner");
    }
    #[test]
    fn body_button_release_is_captured_and_retirement_discards_capture() {
        let mut s = State::new(800, 600).unwrap();
        let h = s.create(1, 448, 288).unwrap();
        s.poll(h).unwrap();
        s.pointer(90, 110, 1);
        while s.poll(h).unwrap().is_some() {}
        s.pointer(799, 599, 0);
        assert_eq!(
            s.poll(h).unwrap(),
            Some(Event::Pointer {
                x: 448,
                y: 288,
                buttons: 0
            })
        );
        s.pointer(90, 110, 1);
        s.retire(h).unwrap();
        let new = s.create(2, 448, 288).unwrap();
        s.poll(new).unwrap();
        s.pointer(799, 599, 0);
        assert_eq!(s.poll(new).unwrap(), None);
    }
    #[test]
    fn hit_focus_drag_release_and_close_exact_content() {
        let mut s = State::new(800, 600).unwrap();
        let a = s.create(10, 448, 288).unwrap();
        let b = s.create(11, 448, 288).unwrap();
        assert_eq!(s.hit(120, 120), Some(b));
        assert!(s.owned(10, a));
        assert!(!s.owned(11, a));
        s.pointer(80, 70, 1);
        assert_eq!(s.focused(), Some(a));
        s.pointer(280, 170, 1);
        assert_eq!((s.find(a).unwrap().x, s.find(a).unwrap().y), (270, 160));
        s.pointer(280, 170, 0);
        s.pointer(0, 0, 0);
        assert_eq!(s.find(a).unwrap().x, 270);
        assert_eq!(s.pointer(706, 170, 1), Action::Close(a));
    }
    #[test]
    fn capacity_stale_cleanup_and_routed_queue() {
        let mut s = State::new(800, 600).unwrap();
        let mut handles = [0; MAX_WINDOWS];
        for (i, h) in handles.iter_mut().enumerate() {
            *h = s.create(100 + i as u64, 448, 288).unwrap();
        }
        let focus = s.focused();
        assert_eq!(s.create(999, 448, 288), Err(Error::Full));
        assert_eq!(s.focused(), focus);
        let old = handles[5];
        assert!(s.key(65));
        s.retire(old).unwrap();
        let new = s.create(999, 448, 288).unwrap();
        assert!(new > old);
        assert_eq!(s.poll(old), Err(Error::Stale));
        assert_eq!(s.poll(new).unwrap(), Some(Event::Focus(true)));
        assert_eq!(s.poll(new).unwrap(), None);
        for i in 0..EVENT_DEPTH {
            assert!(s.send(new, Event::Key(i as u16)));
        }
        assert!(!s.send(new, Event::Key(999)));
        assert_eq!(s.dropped, 1);
        for i in 0..EVENT_DEPTH {
            assert_eq!(s.poll(new).unwrap(), Some(Event::Key(i as u16)));
        }
    }
    #[test]
    fn exhausted_generations_refuse_before_publication() {
        let mut s = State::new(800, 600).unwrap();
        s.z = u64::MAX - 1;
        assert_eq!(s.create(42, 448, 288), Err(Error::Exhausted));
        assert_eq!(s.windows().count(), 0);
        assert_eq!(s.next, 1);
        assert_eq!(s.focused(), None);
        s.z = 1;
        let h = s.create(42, 448, 288).unwrap();
        s.z = u64::MAX;
        assert_eq!(s.activate(h), Err(Error::Exhausted));
        assert_eq!(s.retire(h), Ok(()));
        assert_eq!(s.windows().count(), 0);
    }
    #[test]
    fn dock_half_open_bounds_and_screen_edges() {
        let mut s = State::new(800, 600).unwrap();
        assert_eq!(s.pointer(226, 599, 1), Action::Launch(0));
        s.pointer(226, 599, 0);
        assert_eq!(s.pointer(573, 599, 1), Action::Launch(5));
        s.pointer(573, 599, 0);
        let h = s.create(12, 448, 288).unwrap();
        s.pointer(80, 70, 1);
        s.pointer(i32::MAX, i32::MAX, 1);
        let w = s.find(h).unwrap();
        assert!(w.x < 800 && w.y < 600);
        s.pointer(i32::MIN, i32::MIN, 1);
        let w = s.find(h).unwrap();
        assert!(w.x + w.width as i32 >= m::VISIBLE_TITLE_WIDTH && w.y >= m::SYSTEM_BAR_HEIGHT);
    }
    #[test]
    fn small_window_title_and_close_remain_above_dock() {
        for (width, height) in [(320, 240), (640, 480), (800, 600)] {
            let mut s = State::new(width, height).unwrap();
            let h = s.create(1, 80, 60).unwrap();
            let w = *s.find(h).unwrap();
            s.pointer(w.x + 6, w.y + 10, 1);
            s.pointer(i32::from(width) / 2, i32::MAX, 1);
            s.pointer(i32::from(width) / 2, i32::MAX, 0);
            let w = *s.find(h).unwrap();
            assert!(w.y + m::TITLE_HEIGHT <= i32::from(height) - m::DOCK_HEIGHT);
            let close_x =
                (w.x + i32::from(w.width) - m::CLOSE_WIDTH + 14).clamp(0, i32::from(width) - 1);
            assert_eq!(s.pointer(close_x, w.y + 10, 1), Action::Close(h));
        }
        let mut s = State::new(320, 240).unwrap();
        for i in 0..MAX_WINDOWS {
            let h = s.create(i as u64 + 1, 80, 60).unwrap();
            assert!(s.find(h).unwrap().y + m::TITLE_HEIGHT <= 240 - m::DOCK_HEIGHT);
        }
    }
    fn drain(s: &mut State, h: u64) -> std::vec::Vec<Event> {
        let mut v = std::vec::Vec::new();
        while let Some(e) = s.poll(h).unwrap() {
            v.push(e);
        }
        v
    }
    fn presses(events: &[Event]) -> usize {
        events
            .iter()
            .filter(|e| {
                matches!(e, Event::Pointer { buttons, .. } | Event::PopupPointer { buttons, .. } if buttons & 1 != 0)
            })
            .count()
    }
    fn pointers(events: &[Event]) -> usize {
        events
            .iter()
            .filter(|e| matches!(e, Event::Pointer { .. } | Event::PopupPointer { .. }))
            .count()
    }
    extern crate std;
    #[test]
    fn transient_surfaces_are_owned_focused_stale_safe_and_menu_dismissal_consumes() {
        let mut s = State::new(800, 600).unwrap();
        let a = s.create(10, 448, 288).unwrap();
        let b = s.create(11, 448, 288).unwrap();
        drain(&mut s, a);
        drain(&mut s, b);
        // Only the focused window's own client may open one.
        assert_eq!(
            s.open_popup(10, a, PopupKind::Menu, 10, 40, 160, 120),
            Err(Error::Invalid)
        );
        assert_eq!(
            s.open_popup(10, b, PopupKind::Menu, 10, 40, 160, 120),
            Err(Error::Invalid)
        );
        assert_eq!(
            s.open_popup(11, b, PopupKind::Menu, 10, 40, 300, 300),
            Err(Error::Invalid),
            "over the pixel bound"
        );
        let first = s
            .open_popup(11, b, PopupKind::Menu, 10, 40, 160, 120)
            .unwrap();
        assert!(s.popup_owned(11, first) && !s.popup_owned(10, first));
        let second = s
            .open_popup(11, b, PopupKind::Menu, 20, 50, 160, 120)
            .unwrap();
        assert!(second > first);
        assert!(s.find_popup(first).is_none());
        assert_eq!(s.close_popup(11, first), Err(Error::Stale));
        let (_, p) = s.find_popup(second).unwrap();
        let wb = *s.find(b).unwrap();
        assert_eq!((p.x, p.y), (wb.x + 20, wb.y + 50));
        // Inside: popup-local pointer events to the owner, captured on press.
        s.pointer(p.x + 5, p.y + 7, 0);
        s.pointer(p.x + 5, p.y + 7, 1);
        s.pointer(799, 599, 0);
        assert_eq!(
            drain(&mut s, b),
            [
                Event::PopupPointer {
                    x: 5,
                    y: 7,
                    buttons: 0
                },
                Event::PopupPointer {
                    x: 5,
                    y: 7,
                    buttons: 1
                },
                Event::PopupPointer {
                    x: 160,
                    y: 120,
                    buttons: 0
                }
            ]
        );
        // A press outside the menu only dismisses it: window a is not
        // activated and receives nothing.
        let wa = *s.find(a).unwrap();
        s.pointer(wa.x + 5, wa.y + 100, 1);
        s.pointer(wa.x + 5, wa.y + 100, 0);
        assert_eq!(s.focused(), Some(b));
        assert_eq!(drain(&mut s, b), [Event::Dismissed(second)]);
        assert_eq!(presses(&drain(&mut s, a)), 0);
        assert!(s.find_popup(second).is_none());
        assert_eq!(s.close_popup(11, second), Err(Error::Stale));
        // The next press reaches the window normally.
        s.pointer(wa.x + 5, wa.y + 100, 1);
        assert_eq!(s.focused(), Some(a));
    }
    #[test]
    fn focus_change_move_and_retire_dismiss_menus_but_dialogs_are_modal_and_persist() {
        let mut s = State::new(800, 600).unwrap();
        let a = s.create(10, 448, 288).unwrap();
        let b = s.create(11, 448, 288).unwrap();
        let menu = s
            .open_popup(11, b, PopupKind::Menu, 0, 30, 100, 80)
            .unwrap();
        assert_eq!(
            s.keyboard_action(crate::input_wire::FOCUS_NEXT),
            Action::Changed
        );
        assert_eq!(s.focused(), Some(a));
        assert!(drain(&mut s, b).contains(&Event::Dismissed(menu)));
        // Dialog: modal to its owner, survives focus loss, moves with it.
        let dialog = s
            .open_popup(10, a, PopupKind::Dialog, 100, 80, 240, 120)
            .unwrap();
        drain(&mut s, a);
        let wa = *s.find(a).unwrap();
        s.pointer(wa.x + 5, wa.y + 200, 1);
        s.pointer(wa.x + 5, wa.y + 200, 0);
        assert_eq!(pointers(&drain(&mut s, a)), 0, "owner body input swallowed");
        assert!(s.find_popup(dialog).is_some());
        let wb = *s.find(b).unwrap();
        // b's right strip is not covered by a.
        s.pointer(wb.x + wb.width as i32 - 10, wb.y + 200, 1);
        s.pointer(wb.x + wb.width as i32 - 10, wb.y + 200, 0);
        assert_eq!(s.focused(), Some(b));
        assert!(s.find_popup(dialog).is_some());
        assert!(!drain(&mut s, a).contains(&Event::Dismissed(dialog)));
        // Drag the owner by its title: the dialog keeps its offset.
        let (_, before) = s.find_popup(dialog).unwrap();
        s.pointer(wa.x + 40, wa.y + 10, 1);
        s.pointer(wa.x + 70, wa.y + 30, 1);
        s.pointer(wa.x + 70, wa.y + 30, 0);
        let (_, after) = s.find_popup(dialog).unwrap();
        assert_eq!((after.x - before.x, after.y - before.y), (30, 20));
        // Capture inside the dialog, then the owner dies: nothing leaks to
        // the next window.
        s.pointer(after.x + 3, after.y + 3, 1);
        s.retire(a).unwrap();
        assert!(s.find_popup(dialog).is_none());
        let c = s.create(12, 448, 288).unwrap();
        drain(&mut s, c);
        s.pointer(799, 599, 0);
        assert!(drain(&mut s, c).is_empty());
    }
    #[test]
    fn tooltips_take_no_input_and_any_press_dismisses_without_consuming() {
        let mut s = State::new(800, 600).unwrap();
        let a = s.create(10, 448, 288).unwrap();
        let w = *s.find(a).unwrap();
        let tip = s
            .open_popup(10, a, PopupKind::Tooltip, 40, 60, 120, 24)
            .unwrap();
        drain(&mut s, a);
        let (_, p) = s.find_popup(tip).unwrap();
        s.pointer(p.x + 2, p.y + 2, 0);
        assert_eq!(
            drain(&mut s, a),
            [Event::Pointer {
                x: p.x + 2 - w.x,
                y: p.y + 2 - w.y,
                buttons: 0
            }]
        );
        s.pointer(p.x + 2, p.y + 2, 1);
        let events = drain(&mut s, a);
        assert_eq!(events[0], Event::Dismissed(tip));
        assert_eq!(presses(&events), 1, "the press is not consumed");
    }
    #[test]
    fn transient_surfaces_stay_on_screen() {
        let mut s = State::new(800, 600).unwrap();
        let a = s.create(10, 448, 288).unwrap();
        let h = s
            .open_popup(10, a, PopupKind::Menu, 1000, 700, 200, 150)
            .unwrap();
        let (_, p) = s.find_popup(h).unwrap();
        assert_eq!((p.x + 200, p.y + 150), (800, 600));
        let h = s
            .open_popup(10, a, PopupKind::Menu, -1000, -700, 200, 150)
            .unwrap();
        let (_, p) = s.find_popup(h).unwrap();
        assert_eq!((p.x, p.y), (0, m::SYSTEM_BAR_HEIGHT));
        assert_eq!(
            s.open_popup(10, a, PopupKind::Menu, 1025, 0, 20, 20),
            Err(Error::Invalid)
        );
    }
    #[test]
    fn resize_is_opt_in_bounded_and_configure_is_coalesced() {
        let mut s = State::new(800, 600).unwrap();
        let a = s.create(10, 448, 288).unwrap();
        drain(&mut s, a);
        assert_eq!(
            s.resize(a, 500, 300),
            Err(Error::Invalid),
            "fixed-size client"
        );
        assert_eq!(s.set_resizable(a, 500, 200), Err(Error::Invalid));
        s.set_resizable(a, 320, 200).unwrap();
        assert_eq!(s.resize(a, 500, 300), Ok(true));
        assert_eq!(s.resize(a, 2000, 10), Ok(true));
        let w = *s.find(a).unwrap();
        assert_eq!(
            (w.width, w.height),
            (320, 518).max((800, 200)).min((800, 200))
        );
        assert_eq!(s.max_surface(), (800, 518));
        assert_eq!(
            drain(&mut s, a),
            [Event::Configure {
                width: 800,
                height: 200
            }],
            "one coalesced Configure with the latest size"
        );
        assert_eq!(s.resize(a, 800, 200), Ok(false));
        assert!(drain(&mut s, a).is_empty());
        // Creation is bounded by the work area of the actual screen.
        assert_eq!(s.create(11, 801, 100), Err(Error::Invalid));
        assert_eq!(s.create(11, 800, 519), Err(Error::Invalid));
        assert!(s.create(11, 800, 518).is_ok());
    }
    #[test]
    fn thirty_two_windows_fit_on_screen_and_the_thirty_third_is_refused() {
        for (width, height) in [(800, 600), (640, 480), (1024, 768)] {
            let mut s = State::new(width, height).unwrap();
            for i in 0..MAX_WINDOWS {
                let h = s.create(i as u64 + 1, 448.min(width), 288).unwrap();
                let w = *s.find(h).unwrap();
                assert!(w.x + m::VISIBLE_TITLE_WIDTH <= i32::from(width), "{i}");
                assert!(w.y + m::TITLE_HEIGHT <= i32::from(height) - m::DOCK_HEIGHT);
            }
            assert_eq!(s.create(99, 448, 288), Err(Error::Full));
            assert_eq!(s.windows().count(), 32);
        }
    }
    #[test]
    fn secondary_press_activates_and_reaches_the_window_without_drag_or_capture() {
        let mut s = State::new(800, 600).unwrap();
        let a = s.create(10, 448, 288).unwrap();
        let _b = s.create(11, 448, 288).unwrap();
        drain(&mut s, a);
        let w = *s.find(a).unwrap();
        // a's left strip is not covered by b.
        s.pointer(w.x + 5, w.y + 150, 2);
        assert_eq!(s.focused(), Some(a));
        let events = drain(&mut s, a);
        assert!(events.contains(&Event::Pointer {
            x: 5,
            y: 150,
            buttons: 2
        }));
        s.pointer(w.x + 5, w.y + 10, 0);
        s.pointer(w.x + 5, w.y + 10, 2);
        s.pointer(w.x + 55, w.y + 40, 2);
        assert_eq!(
            (s.find(a).unwrap().x, s.find(a).unwrap().y),
            (w.x, w.y),
            "no drag"
        );
        let menu = s
            .open_popup(10, a, PopupKind::Menu, 5, 150, 100, 60)
            .unwrap();
        s.pointer(w.x + 300, w.y + 250, 0);
        s.pointer(w.x + 300, w.y + 250, 2);
        assert!(drain(&mut s, a).contains(&Event::Dismissed(menu)));
        assert_eq!(s.focused(), Some(a));
    }
    fn resizable(s: &mut State, backing: u64) -> u64 {
        let h = s.create(backing, 448, 288).unwrap();
        s.set_resizable(h, 448, 200).unwrap();
        drain(s, h);
        h
    }
    #[test]
    fn title_drag_has_a_threshold_and_double_click_maximizes_and_restores() {
        let mut s = State::new(800, 600).unwrap();
        let a = resizable(&mut s, 10);
        let w = *s.find(a).unwrap();
        s.set_time(1_000_000);
        s.pointer(w.x + 100, w.y + 10, 1);
        s.pointer(w.x + 102, w.y + 11, 1);
        assert_eq!((s.find(a).unwrap().x, s.find(a).unwrap().y), (w.x, w.y));
        s.pointer(w.x + 102, w.y + 11, 0);
        // Second press 300 ms later at the same place: maximize.
        s.set_time(1_300_000);
        s.pointer(w.x + 101, w.y + 10, 1);
        s.pointer(w.x + 101, w.y + 10, 0);
        let m = *s.find(a).unwrap();
        assert_eq!((m.x, m.y, m.width, m.height), (0, 26, 800, 518));
        assert_eq!(m.placement, Placement::Maximized);
        assert!(drain(&mut s, a).contains(&Event::Configure {
            width: 800,
            height: 518
        }));
        // A late second click is not a double click.
        s.set_time(3_000_000);
        s.pointer(300, 36, 1);
        s.pointer(300, 36, 0);
        s.set_time(3_500_000);
        s.pointer(300, 36, 1);
        s.pointer(300, 36, 0);
        assert_eq!(s.find(a).unwrap().width, 800);
        s.set_time(3_700_000);
        s.pointer(300, 36, 1);
        s.pointer(300, 36, 0);
        let r = *s.find(a).unwrap();
        assert_eq!(
            (r.x, r.y, r.width, r.height),
            (w.x, w.y, 448, 288),
            "exact restore"
        );
        assert_eq!(r.restore, None);
    }
    #[test]
    fn title_buttons_minimize_and_maximize_only_where_supported() {
        let mut s = State::new(800, 600).unwrap();
        let fixed = s.create(9, 448, 288).unwrap();
        let a = resizable(&mut s, 10);
        let wa = *s.find(a).unwrap();
        let right = wa.x + 448;
        assert_eq!(
            State::title_button(&wa, right - 10, wa.y + 10),
            Some(Button::Close)
        );
        assert_eq!(
            State::title_button(&wa, right - 40, wa.y + 10),
            Some(Button::Maximize)
        );
        assert_eq!(
            State::title_button(&wa, right - 70, wa.y + 10),
            Some(Button::Minimize)
        );
        assert_eq!(State::title_button(&wa, right - 100, wa.y + 10), None);
        let wf = *s.find(fixed).unwrap();
        let rf = wf.x + 448;
        assert_eq!(
            State::title_button(&wf, rf - 40, wf.y + 10),
            Some(Button::Minimize)
        );
        // Hover feedback follows the pointer.
        s.pointer(right - 40, wa.y + 10, 0);
        assert_eq!(s.hover, Some((a, Button::Maximize)));
        s.pointer(right - 40, wa.y + 10, 1);
        s.pointer(right - 40, wa.y + 10, 0);
        assert_eq!(s.find(a).unwrap().placement, Placement::Maximized);
        // Minimize: hidden, focus moves on, restorable from the dock policy.
        s.pointer(800 - 70, 36, 1);
        s.pointer(800 - 70, 36, 0);
        assert!(s.find(a).unwrap().minimized);
        assert_eq!(s.focused(), Some(fixed));
        assert_eq!(
            s.hit(400, 200),
            Some(fixed).filter(|_| s.find(fixed).unwrap().contains(400, 200))
        );
        assert_eq!(s.minimized_window(|b| b == 10), Some(a));
        s.activate(a).unwrap();
        assert!(!s.find(a).unwrap().minimized);
        assert_eq!(s.find(a).unwrap().width, 800, "placement survives minimize");
        // Fixed-size windows refuse every placement.
        assert_eq!(s.maximize(fixed), Err(Error::Invalid));
        assert_eq!(s.snap_to(fixed, Placement::Left), Err(Error::Invalid));
    }
    #[test]
    fn frame_edges_resize_with_minimum_work_area_and_bar_clamps() {
        let mut s = State::new(800, 600).unwrap();
        let a = resizable(&mut s, 10);
        let w = *s.find(a).unwrap();
        let (x1, y1) = (w.x + 448, w.y + 288);
        // Right edge, just outside the frame.
        s.pointer(x1 + 3, w.y + 100, 1);
        s.pointer(x1 + 53, w.y + 140, 1);
        s.pointer(x1 + 53, w.y + 140, 0);
        let r = *s.find(a).unwrap();
        assert_eq!((r.x, r.y, r.width, r.height), (w.x, w.y, 498, 288));
        // Bottom-left corner: grow left and down.
        s.pointer(w.x - 2, y1 + 2, 1);
        s.pointer(w.x - 32, y1 + 22, 1);
        s.pointer(w.x - 32, y1 + 22, 0);
        let r = *s.find(a).unwrap();
        assert_eq!((r.x, r.width, r.height), (w.x - 30, 528, 308));
        // Shrinking stops at the declared minimum.
        let r0 = r;
        s.pointer(r0.x + r0.width as i32 + 2, r0.y + 100, 1);
        s.pointer(0, 0, 1);
        s.pointer(0, 0, 0);
        assert_eq!(s.find(a).unwrap().width, 448);
        // Top edge cannot pull the frame under the system bar.
        let r = *s.find(a).unwrap();
        s.pointer(r.x + 100, r.y - 3, 1);
        s.pointer(r.x + 100, 0, 1);
        s.pointer(r.x + 100, 0, 0);
        let t = *s.find(a).unwrap();
        assert_eq!(t.y, m::SYSTEM_BAR_HEIGHT);
        assert_eq!(t.y + t.height as i32, r.y + r.height as i32);
        // Every size change reached the client as (coalesced) Configure.
        let configures = drain(&mut s, a)
            .into_iter()
            .filter(|e| matches!(e, Event::Configure { .. }))
            .count();
        assert!(configures >= 3);
        // Fixed-size windows have no resize zone.
        let f = s.create(11, 448, 288).unwrap();
        let wf = *s.find(f).unwrap();
        s.pointer(wf.x + 448 + 3, wf.y + 100, 1);
        s.pointer(wf.x + 448 + 60, wf.y + 100, 1);
        s.pointer(wf.x + 448 + 60, wf.y + 100, 0);
        assert_eq!(s.find(f).unwrap().width, 448);
    }
    #[test]
    fn drag_to_edges_snaps_and_keyboard_places_restores_and_minimizes() {
        let mut s = State::new(800, 600).unwrap();
        let a = resizable(&mut s, 10);
        let w = *s.find(a).unwrap();
        s.pointer(w.x + 100, w.y + 10, 1);
        s.pointer(200, 200, 1);
        s.pointer(0, 200, 1);
        // The declared 448 minimum exceeds half of 800: halves clamp to it.
        assert_eq!(s.snap_preview(), Some((0, 26, 448, 518)));
        s.pointer(0, 200, 0);
        let l = *s.find(a).unwrap();
        assert_eq!(
            (l.x, l.y, l.width, l.height, l.placement),
            (0, 26, 448, 518, Placement::Left)
        );
        assert_eq!(s.snap_preview(), None);
        let sup = MOD_SUPER;
        s.key_input(257, true, sup);
        let r = *s.find(a).unwrap();
        assert_eq!((r.x, r.width, r.placement), (352, 448, Placement::Right));
        s.key_input(258, true, sup);
        assert_eq!(s.find(a).unwrap().placement, Placement::Maximized);
        s.key_input(259, true, sup);
        let back = *s.find(a).unwrap();
        assert_eq!((back.width, back.height, back.restore), (448, 288, None));
        s.key_input(259, true, sup);
        assert!(s.find(a).unwrap().minimized);
        // Dragging a maximized window out restores its free size.
        s.activate(a).unwrap();
        s.maximize(a).unwrap();
        s.pointer(400, 36, 1);
        s.pointer(420, 120, 1);
        s.pointer(420, 120, 0);
        let d = *s.find(a).unwrap();
        assert_eq!(
            (d.width, d.placement, d.restore),
            (back.width, Placement::Free, None)
        );
    }
    #[test]
    fn key_repeat_chords_and_host_autorepeat_do_not_stack() {
        let mut s = State::new(800, 600).unwrap();
        let a = s.create(10, 448, 288).unwrap();
        drain(&mut s, a);
        s.set_time(0);
        s.key_input(u16::from(b'a'), true, 0);
        assert_eq!(s.repeat_deadline(), Some(REPEAT_DELAY_US));
        assert!(!s.repeat_tick(REPEAT_DELAY_US - 1));
        assert!(s.repeat_tick(REPEAT_DELAY_US));
        assert!(s.repeat_tick(REPEAT_DELAY_US + REPEAT_INTERVAL_US));
        s.key_input(u16::from(b'a'), false, 0);
        assert_eq!(s.repeat_deadline(), None);
        assert_eq!(
            drain(&mut s, a),
            [Event::Key(97), Event::Key(97), Event::Key(97)]
        );
        // Host autorepeat (repeated presses) restarts the delay instead.
        s.set_time(1_000_000);
        s.key_input(u16::from(b'b'), true, 0);
        s.set_time(1_030_000);
        s.key_input(u16::from(b'b'), true, 0);
        assert_eq!(s.repeat_deadline(), Some(1_030_000 + REPEAT_DELAY_US));
        s.key_input(u16::from(b'b'), false, 0);
        // Shortcuts arrive as chords; Shift alone is an ordinary key.
        s.key_input(u16::from(b's'), true, MOD_CTRL);
        s.key_input(u16::from(b'S'), true, MOD_SHIFT);
        let events = drain(&mut s, a);
        assert!(events.contains(&Event::Chord {
            code: 115,
            mods: MOD_CTRL
        }));
        assert!(events.contains(&Event::Key(83)));
    }
    #[test]
    fn alt_tab_switcher_cycles_most_recent_first_commits_on_alt_release_and_cancels() {
        use crate::input_wire::MODIFIERS;
        let mut s = State::new(800, 600).unwrap();
        let a = s.create(10, 448, 288).unwrap();
        let b = s.create(11, 448, 288).unwrap();
        let c = s.create(12, 448, 288).unwrap();
        assert_eq!(s.cycle_order().0[..3], [c, b, a]);
        s.key_input(9, true, MOD_ALT);
        assert_eq!(s.switcher().map(|(sel, o, _)| o[sel]), Some(b));
        s.key_input(9, true, MOD_ALT);
        assert_eq!(s.switcher().map(|(sel, o, _)| o[sel]), Some(a));
        s.key_input(9, true, MOD_ALT | MOD_SHIFT);
        assert_eq!(s.switcher().map(|(sel, o, _)| o[sel]), Some(b));
        s.key_input(MODIFIERS, true, 0);
        assert_eq!(s.focused(), Some(b));
        assert!(s.switcher().is_none());
        // Escape cancels without changing focus; a minimized window is
        // restored when chosen.
        s.minimize(a).unwrap();
        s.key_input(9, true, MOD_ALT);
        s.key_input(27, true, MOD_ALT);
        s.key_input(MODIFIERS, true, 0);
        assert_eq!(s.focused(), Some(b));
        s.key_input(9, true, MOD_ALT);
        s.key_input(9, true, MOD_ALT);
        s.key_input(MODIFIERS, true, 0);
        assert_eq!(s.focused(), Some(a));
        assert!(!s.find(a).unwrap().minimized);
    }
    #[test]
    fn wheel_goes_to_the_window_under_the_pointer() {
        let mut s = State::new(800, 600).unwrap();
        let a = s.create(10, 448, 288).unwrap();
        let w = *s.find(a).unwrap();
        drain(&mut s, a);
        s.pointer(w.x + 30, w.y + 100, 0);
        drain(&mut s, a);
        s.wheel(-2);
        assert_eq!(
            drain(&mut s, a),
            [Event::Wheel {
                x: 30,
                y: 100,
                delta: -2
            }]
        );
        s.pointer(799, 599, 0);
        s.wheel(1);
        assert!(drain(&mut s, a).is_empty());
    }
}
