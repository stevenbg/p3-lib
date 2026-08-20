use crate::data::p3_ptr::P3Pointer;

pub const STATIC_UI_TAVERN_WINDOW_PTR_ADDRESS: *const u32 = 0x006E5574 as _;

/// The tavern window, vtable `0x00679B78`: open `+0x120` = `0x005CC120`, close
/// `+0x118` = `0x005CD2A0`, per-frame update `+0xF4` = `0x005CD540`, draw `+0x9C` =
/// `0x005CDC60`.
///
/// Constructed once at startup like the other building windows: the mass-constructor
/// calls the constructor (`0x005CB9B0`) at `0x00426C2C` and stores the returned `this`
/// into the static at `0x00426C45`; the shutdown path destructs the object and nulls
/// the static at `0x00427F74`. The window's own methods never read the static - the
/// scrollmap does, e.g. at `0x005A650F` to call the open method - which is why looking
/// for the static inside the window's code range finds nothing.
#[derive(Clone, Debug, Copy)]
pub struct UITavernWindowPtr {
    pub address: u32,
}

impl Default for UITavernWindowPtr {
    fn default() -> Self {
        Self::new()
    }
}

impl UITavernWindowPtr {
    pub const VTABLE_OFFSET: u32 = 0x279B78;

    pub fn new() -> Self {
        Self {
            address: unsafe { *STATIC_UI_TAVERN_WINDOW_PTR_ADDRESS },
        }
    }

    pub fn get_x(&self) -> i32 {
        unsafe { self.get(0x14) }
    }

    pub fn get_y(&self) -> i32 {
        unsafe { self.get(0x18) }
    }

    pub fn get_width(&self) -> i32 {
        unsafe { self.get(0x2c) }
    }

    pub fn get_height(&self) -> i32 {
        unsafe { self.get(0x30) }
    }

    /// The town the tavern belongs to. The window computes the town address from it at
    /// `0x005CC931` and passes it as the town index to the sailor-availability getter at
    /// `0x005CD4C1`.
    pub unsafe fn get_town_index(&self) -> u16 {
        self.get(0x1bfc)
    }

    /// The tavern's page, `-1` being the empty page the window starts on. Both the
    /// update method (page load at `0x005CD54D`, `this` in edi) and the draw method
    /// (page load at `0x005CE3E0`, `this` in esi) load this field and dispatch through
    /// a jump table of 18 entries guarded by `cmp eax, 0x11 / ja`, so `-1` skips every
    /// page's drawing. The 18 entries are what the dispatch can reach, not what the
    /// window offers: which pages have a tab depends on the game state (the captain
    /// and pirate pages, for instance, only while one is available in that town).
    pub unsafe fn get_selected_page(&self) -> i32 {
        self.get(0x1bf4)
    }
}

impl P3Pointer for UITavernWindowPtr {
    fn get_address(&self) -> u32 {
        self.address
    }
}
