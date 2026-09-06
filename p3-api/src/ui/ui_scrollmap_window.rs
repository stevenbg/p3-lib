//! The **scrollmap** - the world-map scene window, the counterpart of
//! [crate::ui::ui_local_map_window::UILocalMapWindowPtr]. Constructed at startup and kept in
//! the static below (`0x00424FB8`); the game swaps between the two scenes by hiding one
//! (`+0xCC`) and disabling it (`+0x34`) and showing the other, e.g. on entering a town from
//! the world map (`0x0044A1BC`..`0x0044A1F4`) or the home town at session start
//! (`0x00433A1B`..`0x00433A52`).

use crate::data::p3_ptr::P3Pointer;

pub const SCROLLMAP_WINDOW_PTR_ADDRESS: *const u32 = 0x006CBDD8 as _;

/// The world-map positions of the towns, one record of [WORLD_MAP_POSITION_STRIDE] bytes
/// per town index: `x` word at `+0x0`, `y` word at `+0x4`. Runtime data (`.data` tail), read
/// by the home-town entry at `0x004339E6`/`0x00433A0D` to centre the map on the town.
pub const WORLD_MAP_POSITIONS_ADDRESS: u32 = 0x006DDBA0;
pub const WORLD_MAP_POSITION_STRIDE: u32 = 0x34;

#[derive(Clone, Debug)]
pub struct UIScrollmapWindowPtr {
    pub address: u32,
}

impl Default for UIScrollmapWindowPtr {
    fn default() -> Self {
        Self::new()
    }
}

impl UIScrollmapWindowPtr {
    /// `0x0044F490(x, y)`, `thiscall`: scroll so that the world-map point is centred -
    /// writes `x - width/2` / `y - height/2` into the scroll offsets `+0x29E4`/`+0x29E6`.
    pub const CENTER_ON_ADDRESS: u32 = 0x0044F490;

    pub fn new() -> Self {
        Self {
            address: unsafe { *SCROLLMAP_WINDOW_PTR_ADDRESS },
        }
    }

    pub fn get_x(&self) -> i32 {
        unsafe { self.get(0x14) }
    }

    pub fn get_y(&self) -> i32 {
        unsafe { self.get(0x18) }
    }

    /// The scroll offsets: the world-map coordinate at the window's top-left corner.
    pub fn get_offset_x(&self) -> i16 {
        unsafe { self.get(0x29e4) }
    }

    pub fn get_offset_y(&self) -> i16 {
        unsafe { self.get(0x29e6) }
    }

    /// Centre the world map on a point (see [Self::CENTER_ON_ADDRESS]).
    pub unsafe fn center_on(&self, x: u16, y: u16) {
        let center: extern "thiscall" fn(this: u32, x: u32, y: u32) = std::mem::transmute(Self::CENTER_ON_ADDRESS);
        center(self.address, x as u32, y as u32);
    }

    /// Centre the world map on a town's position from [WORLD_MAP_POSITIONS_ADDRESS] - what
    /// the game does before it hands the screen to the town, so the world map is on the
    /// town when the player comes back.
    pub unsafe fn center_on_town(&self, town_index: u8) {
        let record = WORLD_MAP_POSITIONS_ADDRESS + town_index as u32 * WORLD_MAP_POSITION_STRIDE;
        let x = *(record as *const u16);
        let y = *((record + 4) as *const u16);
        self.center_on(x, y);
    }
}

impl P3Pointer for UIScrollmapWindowPtr {
    fn get_address(&self) -> u32 {
        self.address
    }
}
