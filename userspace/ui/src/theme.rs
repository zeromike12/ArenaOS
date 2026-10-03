//! Engineering reference palettes. These tokens never enter service wires.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    pub background: u32,
    pub panel: u32,
    pub elevated: u32,
    pub text: u32,
    pub secondary: u32,
    pub accent: u32,
    pub error: u32,
    pub warning: u32,
    pub selection: u32,
    pub border: u32,
    pub chrome_active: u32,
    pub chrome_inactive: u32,
    pub terminal: u32,
    pub terminal_text: u32,
    pub disabled: u32,
}
pub const LIGHT: Theme = Theme {
    background: 0x283e51,
    panel: 0xe9edf1,
    elevated: 0xfafafa,
    text: 0x202731,
    secondary: 0x616f7c,
    accent: 0x426aa3,
    error: 0xa84242,
    warning: 0x946f27,
    selection: 0xd5e3f4,
    border: 0xabb5c0,
    chrome_active: 0xdce6f2,
    chrome_inactive: 0xe9edf1,
    terminal: 0x202831,
    terminal_text: 0xe4ece4,
    disabled: 0x9ba3ad,
};
pub const DARK: Theme = Theme {
    background: 0x182532,
    panel: 0x2d3541,
    elevated: 0x222b35,
    text: 0xe6edf4,
    secondary: 0xa4b3c2,
    accent: 0x91b5e8,
    error: 0xe38585,
    warning: 0xe0be77,
    selection: 0x3e5876,
    border: 0x526174,
    chrome_active: 0x3e5169,
    chrome_inactive: 0x2d3541,
    terminal: 0x151c25,
    terminal_text: 0xd7e6d7,
    disabled: 0x727f90,
};
pub const fn palette(dark: bool) -> Theme {
    if dark { DARK } else { LIGHT }
}
