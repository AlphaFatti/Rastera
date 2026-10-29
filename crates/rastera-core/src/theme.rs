//! Rastera tokens. Document content colors are independent of UI styling.
pub const WORKSPACE: [u8; 4] = [0x17, 0x15, 0x1A, 255];
pub const DEEP_PLUM: [u8; 4] = [0x41, 0x11, 0x41, 255];
pub const RASTER_PURPLE: [u8; 4] = [0x5f, 0x0e, 0x61, 255];
pub const SIGNAL_MINT: [u8; 4] = [0x26, 0xd0, 0x85, 255];
pub const ELECTRIC_YELLOW: [u8; 4] = [255, 254, 0, 255];
pub const SIGNAL_WHITE: [u8; 4] = [255; 4];
pub const HOT_MAGENTA: [u8; 4] = [255, 0, 128, 255];
pub const ELECTRIC_FUCHSIA: [u8; 4] = [255, 0, 255, 255];
pub const RETRO_LAVENDER: [u8; 4] = [170, 85, 170, 255];
pub const BOUNDARY: [u8; 4] = [218, 213, 222, 255];
pub fn floats(c: [u8; 4]) -> [f32; 4] {
    c.map(|n| n as f32 / 255.)
}
