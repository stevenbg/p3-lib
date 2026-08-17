use std::mem;

use crate::data::p3_ptr::P3Pointer;

pub const STATIC_UI_TRADING_OFFICE_WINDOW_PTR_ADDRESS: *const u32 = 0x006E557C as _;

#[derive(Clone, Debug, Copy)]
pub struct UITradingOfficeWindowPtr {
    address: u32,
}

impl Default for UITradingOfficeWindowPtr {
    fn default() -> Self {
        Self::new()
    }
}

impl UITradingOfficeWindowPtr {
    pub const VTABLE_OFFSET: u32 = 0x279CB0;
    pub fn new() -> Self {
        Self {
            address: unsafe { *STATIC_UI_TRADING_OFFICE_WINDOW_PTR_ADDRESS },
        }
    }

    /// The side menu view: 0 Total, ..., 4 Trading Office (administrator), ...
    pub unsafe fn get_selected_page(&self) -> i32 {
        self.get(0xecc4)
    }

    pub unsafe fn get_town_index(&self) -> u16 {
        self.get(0xecc8)
    }

    /// Switches the side menu view and rebuilds its widgets. Re-selecting the current
    /// page forces a redraw after changing underlying data.
    pub unsafe fn select_new_page(&self, new_page: i32) {
        let set_new_page: extern "thiscall" fn(this: u32, new_page: i32) = mem::transmute(0x005D9A20);
        set_new_page(self.address, new_page)
    }
}

impl P3Pointer for UITradingOfficeWindowPtr {
    fn get_address(&self) -> u32 {
        self.address
    }
}
