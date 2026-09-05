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
    pub fn new() -> Self {
        let ptr: *const u32 = CLASS48_PTR_ADDRESS as _;
        Self { address: unsafe { *ptr } }
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
