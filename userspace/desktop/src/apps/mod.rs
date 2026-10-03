pub mod layout;
pub mod model;
pub mod view;
pub const TERMINAL: u8 = 0;
pub const FILES: u8 = 1;
pub const EDITOR: u8 = 2;
pub const SETTINGS: u8 = 3;
pub const MONITOR: u8 = 4;
pub const GALLERY: u8 = 5;
/// Descriptive window titles (at most 32 bytes); they confer no authority.
pub const TITLES: [&str; 6] = [
    "Terminal",
    "Files",
    "Text Editor",
    "Settings",
    "System Monitor",
    "UI Gallery",
];
/// Dock labels, each at most 8 characters so they fit one dock slot.
pub const DOCK: [&str; 6] = [
    "Terminal", "Files", "Editor", "Settings", "Monitor", "Gallery",
];
