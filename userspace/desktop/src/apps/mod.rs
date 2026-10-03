pub mod layout;
pub mod model;
pub mod view;
pub const TERMINAL: u8 = 0;
pub const FILES: u8 = 1;
pub const EDITOR: u8 = 2;
pub const SETTINGS: u8 = 3;
pub const MONITOR: u8 = 4;
pub const GALLERY: u8 = 5;
pub const TITLES: [&str; 6] = [
    "ArenaOS Terminal",
    "Files",
    "Text Editor",
    "Settings",
    "System Monitor",
    "ArenaOS UI Gallery",
];
pub const DOCK: [&str; 6] = ["TERM", "FILES", "EDIT", "SETUP", "STATS", "UI"];
