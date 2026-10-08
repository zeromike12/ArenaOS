pub mod explorer;
pub mod explorer_ctl;
pub mod explorer_view;
pub mod layout;
pub mod model;
pub mod scene;
pub mod view;
pub const TERMINAL: u8 = 0;
pub const FILES: u8 = 1;
pub const EDITOR: u8 = 2;
pub const SETTINGS: u8 = 3;
pub const MONITOR: u8 = 4;
pub const GALLERY: u8 = 5;

const fn application_id(bytes: &[u8]) -> [u8; 32] {
    let mut out = [0; 32];
    let mut i = 0;
    while i < bytes.len() {
        out[i] = bytes[i];
        i += 1;
    }
    out
}
/// Canonical descriptive IDs used in ABI-v2 startup records. These are not
/// launch authority; the trusted Desktop image selection grants authority.
pub const APPLICATION_IDS: [[u8; 32]; 6] = [
    application_id(b"org.arenaos.terminal"),
    application_id(b"org.arenaos.files"),
    application_id(b"org.arenaos.editor"),
    application_id(b"org.arenaos.settings"),
    application_id(b"org.arenaos.monitor"),
    application_id(b"org.arenaos.gallery"),
];
/// All Desktop static images use `desktop.ld` and this fixed entry/load base.
/// The application runtime compares the startup metadata with its actual
/// `_start` address before entering application code.
pub const APPLICATION_ENTRY: u64 = 0x0020_0000;
pub const APPLICATION_LOAD_BASE: u64 = 0x0020_0000;
/// ABI-0083's bounded app-instance slot domain, shared with the lifecycle
/// table. The independent ordinary-window/session limit remains 32.
pub const STARTUP_INSTANCE_SLOTS: usize = arena_startup_abi::startup::INSTANCE_SLOTS;

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
