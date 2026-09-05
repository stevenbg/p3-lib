use std::{ffi::c_void, mem};

use super::p3_ptr::P3Pointer;

pub const CLASS48_PTR_ADDRESS: u32 = 0x006CC7E0;

#[derive(Clone, Debug)]
pub struct Class48Ptr {
    pub address: u32,
}

impl Default for Class48Ptr {
    fn default() -> Self {
        Self::new()
    }
}

impl Class48Ptr {
    /// Vtable `0x0066E688`, module-relative. Draw is `+0x9C` = `0x00465BA0`
    /// (`thiscall(context, x, y, z)`, `ret 0x10`): it blits the current building's picture
    /// at the inset, then the whitening veil over it.
    pub const VTABLE_OFFSET: u32 = 0x26E688;
    /// The building picture and the veil are drawn this far inside the object's top-left
    /// corner (`0x00465C67`, `0x00465EA0`); the veil is the object's width minus 26 wide and
    /// its height minus the gradient y minus 27 tall (`0x00465E85`..`0x00465E91`).
    pub const PICTURE_INSET: i32 = 12;
    pub const VEIL_WIDTH_MARGIN: i32 = 26;
    pub const VEIL_HEIGHT_MARGIN: i32 = 27;
    /// The colour the veil is blitted with: white at alpha 160 (`0x00465E73`).
    pub const VEIL_COLOR: u32 = 0xA0FF_FFFF;
    /// The two `0x004BB330` calls that blit the veil, module-relative, for `hook_call_rel32`:
    /// the one-row ramp blits and the flat sheet. Both take their size from the object, and
    /// the texture behind them is `pergament.aim`, 425 x 510 (`BackTexID` of
    /// `animation.ini`): an object taller than 537 makes the flat blit read past the texture
    /// and crash in `ddraw_dll`, so an enlarged backdrop has to tile these blits.
    pub const VEIL_RAMP_BLIT_CALL_OFFSET: u32 = 0x065E45;
    pub const VEIL_FLAT_BLIT_CALL_OFFSET: u32 = 0x065EA8;
    /// Above the flat veil, when the gradient y is non-zero, the draw blits this many one-row
    /// strips of white with alpha 0..159 (`0x00465E52`): row `i` lands at
    /// `y + gradient_y + i - VEIL_RAMP_OFFSET` (`0x00465E14`), skipped while it is not below
    /// the object's top edge or while `gradient_y + i < VEIL_RAMP_ROWS` (`0x00465E26`).
    pub const VEIL_RAMP_ROWS: i32 = 160;
    pub const VEIL_RAMP_OFFSET: i32 = 148;

    pub fn new() -> Self {
        let ptr: *const u32 = CLASS48_PTR_ADDRESS as _;
        Self { address: unsafe { *ptr } }
    }

    /// The renderer handle of the current building's picture: `+0x94` is an array of
    /// graphic records, one per `[AnimN]` of `animation.ini`, indexed by the current
    /// building at `+0xE8` (`0x00465C30`..`0x00465C5C`); the handle is the record's `+0x4`.
    pub unsafe fn current_picture_handle(&self) -> Option<u32> {
        let records: u32 = self.get(0x94);
        let index: u32 = self.get(0xe8);
        if records == 0 {
            return None;
        }
        let record = *((records + index * 4) as *const u32);
        if record == 0 {
            return None;
        }
        let handle = *((record + 4) as *const u32);
        (handle != 0).then_some(handle)
    }

    /// The handle of the white texture the veil is blitted from (`+0xF8`, `0x00465E67`).
    pub unsafe fn veil_texture_handle(&self) -> u32 {
        self.get(0xf8)
    }

    /// The graphic record of the wooden frame's eight pieces (`+0x9C`, from the `RahmenID`
    /// key of `animation.ini`): frames 0..3 are the corners top-left, top-right,
    /// bottom-right, bottom-left, blitted at the object's corners; 4 and 5 the top and
    /// bottom bar pieces, tiled [Self::BAR_TILES] times from `x + 12` (`0x00466133`,
    /// `0x004661B1`); 6 and 7 the right and left bar pieces, tiled 34 times from `y + 12`.
    /// The tile counts are constants, so a wider object leaves the horizontal bars short.
    pub unsafe fn frame_record(&self) -> u32 {
        self.get(0x9c)
    }

    pub const FRAME_CORNER_TOP_LEFT: u32 = 0;
    pub const FRAME_CORNER_TOP_RIGHT: u32 = 1;
    pub const FRAME_CORNER_BOTTOM_RIGHT: u32 = 2;
    pub const FRAME_CORNER_BOTTOM_LEFT: u32 = 3;
    pub const FRAME_TOP_BAR: u32 = 4;
    pub const FRAME_BOTTOM_BAR: u32 = 5;
    /// Tiled down the right edge, at `x + width - piece width` (`0x004662C7`).
    pub const FRAME_RIGHT_BAR: u32 = 6;
    /// Tiled down the left edge, at `x` (`0x0046624B`).
    pub const FRAME_LEFT_BAR: u32 = 7;
    /// Tiles of the top and bottom bar the draw blits (`0x11`).
    pub const BAR_TILES: i32 = 17;
    /// Tiles of the left and right bar the draw blits (`0x22`, `0x00466236`, `0x004662B2`).
    pub const SIDE_BAR_TILES: i32 = 34;

    pub fn gradient_y(&self) -> u16 {
        unsafe { self.get(0xec) }
    }

    /// Where the white veil over the building animation begins, as a y relative to the
    /// window, read by the background pass at `0x00465DC0`. When non-zero, a 160-row ramp is
    /// drawn above it - row `i` at `window.y + value + i - 148` in `(i << 24) | 0xFFFFFF`, so
    /// alpha 0..159 rising towards the veil - and, unless [Self::set_ignore_below_gradient]
    /// is set, the rows from `value + 12` to the bottom are filled with `0xA0FFFFFF`, a flat
    /// white at alpha 160. With 0 the ramp lies above the window and the whole window is
    /// veiled evenly. The game's pages set it per page (0 to 464 at the writers).
    pub fn set_gradient_y(&self, value: u16) {
        let ptr: *mut u16 = (self.address + 0xec) as _;
        unsafe { *ptr = value }
    }

    /// Non-zero skips the flat veil below the gradient y, leaving only the ramp
    /// (`0x00465E5D`).
    pub fn set_ignore_below_gradient(&self, value: u8) {
        let ptr: *mut u8 = (self.address + 0xfc) as _;
        unsafe { *ptr = value }
    }

    pub unsafe fn clip_stuff(&self) {
        let function: extern "thiscall" fn(this: *const c_void) = unsafe { mem::transmute(0x004665D0) };
        function(self.address as _)
    }
}

impl P3Pointer for Class48Ptr {
    fn get_address(&self) -> u32 {
        self.address
    }
}
