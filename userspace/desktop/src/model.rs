//! Window policy holds descriptive handles. The service supplies authority
//! from its own exact backing / held Process associations, never caller IDs.
use arena_ui::metrics as m;
pub const MAX_WINDOWS: usize = 6;
pub const EVENT_DEPTH: usize = 32;
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
    Pointer { x: i32, y: i32, buttons: u8 },
    Close,
    Focus(bool),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    Launch(usize),
    Close(u64),
    Changed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    pub handle: u64,
    pub backing: u64,
    pub x: i32,
    pub y: i32,
    pub width: u16,
    pub height: u16,
    pub z: u64,
    pub minimized: bool,
    queue: [Option<Event>; EVENT_DEPTH],
    head: usize,
    len: usize,
}
impl Window {
    fn enqueue(&mut self, e: Event) -> Result<(), Error> {
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
pub struct State {
    windows: [Option<Window>; MAX_WINDOWS],
    next: u64,
    z: u64,
    focused: Option<u64>,
    drag: Option<(u64, i32, i32)>,
    capture: Option<u64>,
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
            drag: None,
            capture: None,
            buttons: 0,
            pointer: (0, 0),
            screen: (width as i32, height as i32),
            dropped: 0,
            dock_items: MAX_WINDOWS,
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
            || dock_items > 8
        {
            return Err(Error::Invalid);
        }
        self.screen = (width as i32, height as i32);
        self.dock_items = dock_items;
        Ok(())
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
    pub fn owned(&self, backing: u64, handle: u64) -> bool {
        self.find(handle).is_some_and(|w| w.backing == backing)
    }
    fn slot(&self, handle: u64) -> Result<usize, Error> {
        self.windows
            .iter()
            .position(|w| w.is_some_and(|w| w.handle == handle))
            .ok_or(Error::Stale)
    }
    pub fn create(&mut self, backing: u64, width: u16, height: u16) -> Result<u64, Error> {
        if backing == 0
            || self.windows().any(|w| w.backing == backing)
            || width < 80
            || height < 60
            || width > m::WINDOW_WIDTH as u16
            || height > m::WINDOW_HEIGHT as u16
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
        self.windows[i] = Some(Window {
            handle,
            backing,
            x: m::WINDOW_START_X + (i as i32) * m::CASCADE_X,
            y: (m::WINDOW_START_Y + (i as i32) * m::CASCADE_Y)
                .clamp(m::SYSTEM_BAR_HEIGHT, self.max_title_y()),
            width,
            height,
            z: self.z,
            minimized: false,
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
        self.windows[i] = None;
        if self.drag.is_some_and(|d| d.0 == handle) {
            self.drag = None;
        }
        if self.capture == Some(handle) {
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
    pub fn poll(&mut self, handle: u64) -> Result<Option<Event>, Error> {
        let i = self.slot(handle)?;
        Ok(self.windows[i].as_mut().ok_or(Error::Stale)?.pop())
    }
    pub fn key(&mut self, key: u16) -> bool {
        self.focused.is_some_and(|h| self.send(h, Event::Key(key)))
    }
    pub fn keyboard_action(&mut self, key: u16) -> Action {
        use crate::input_wire as input;
        match key {
            input::LAUNCH_FIRST..=input::LAUNCH_LAST => {
                Action::Launch((key - input::LAUNCH_FIRST) as usize)
            }
            input::CLOSE_FOCUSED => self.focused.map(Action::Close).unwrap_or(Action::None),
            input::FOCUS_NEXT => {
                let mut order = [(0u64, 0u64); MAX_WINDOWS];
                let mut count = 0;
                for w in self.windows() {
                    order[count] = (w.z, w.handle);
                    count += 1;
                }
                order[..count].sort_unstable();
                if count > 0 && self.activate(order[0].1).is_ok() {
                    Action::Changed
                } else {
                    Action::None
                }
            }
            _ => {
                self.key(key);
                Action::None
            }
        }
    }
    pub fn hit(&self, x: i32, y: i32) -> Option<u64> {
        self.windows()
            .filter(|w| w.contains(x, y))
            .max_by_key(|w| w.z)
            .map(|w| w.handle)
    }
    pub fn pointer(&mut self, x: i32, y: i32, buttons: u8) -> Action {
        let (x, y) = (x.clamp(0, self.screen.0 - 1), y.clamp(0, self.screen.1 - 1));
        let pressed = buttons & 1 != 0 && self.buttons & 1 == 0;
        let released = buttons & 1 == 0 && self.buttons & 1 != 0;
        self.pointer = (x, y);
        self.buttons = buttons & 7;
        if released {
            self.drag = None;
        }
        if let Some((handle, dx, dy)) = self.drag {
            let max_y = self.max_title_y();
            if let Ok(i) = self.slot(handle) {
                let w = self.windows[i].as_mut().expect("checked slot");
                w.x = (x - dx).clamp(
                    m::VISIBLE_TITLE_WIDTH - w.width as i32,
                    self.screen.0 - m::VISIBLE_TITLE_WIDTH,
                );
                w.y = (y - dy).clamp(m::SYSTEM_BAR_HEIGHT, max_y);
                return Action::Changed;
            }
            self.drag = None;
        }
        if let Some(h) = self.capture {
            if let Some(w) = self.find(h).copied() {
                self.send(
                    h,
                    Event::Pointer {
                        x: (x - w.x).clamp(-(w.width as i32), w.width as i32),
                        y: (y - w.y).clamp(-(w.height as i32), w.height as i32),
                        buttons: buttons & 7,
                    },
                );
            }
            if released {
                self.capture = None;
            }
            return Action::Changed;
        }
        let dock_w = self.dock_items as i32 * m::DOCK_ITEM_WIDTH;
        let dock_x = (self.screen.0 - dock_w) / 2;
        if pressed && y >= self.screen.1 - m::DOCK_HEIGHT && x >= dock_x && x < dock_x + dock_w {
            return Action::Launch(((x - dock_x) / m::DOCK_ITEM_WIDTH) as usize);
        }
        if let Some(h) = self.hit(x, y) {
            if pressed {
                if self.activate(h).is_err() {
                    return Action::None;
                }
                let w = *self.find(h).expect("activated window");
                if y < w.y + m::TITLE_HEIGHT {
                    if x >= w.x + w.width as i32 - m::CLOSE_WIDTH {
                        return Action::Close(h);
                    }
                    self.drag = Some((h, x - w.x, y - w.y));
                    return Action::Changed;
                }
                self.capture = Some(h);
            }
            let w = *self.find(h).expect("hit window");
            self.send(
                h,
                Event::Pointer {
                    x: x - w.x,
                    y: y - w.y,
                    buttons: buttons & 7,
                },
            );
        }
        Action::Changed
    }
}
#[cfg(test)]
mod tests {
    use super::*;
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
}
