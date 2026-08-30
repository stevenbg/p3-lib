use crate::data::p3_ptr::P3Pointer;

/// The church window's global, established for `mod-fix-church-anim-crash`.
pub const STATIC_UI_CHURCH_WINDOW_PTR_ADDRESS: *const u32 = 0x006E556C as _;

/// The church window, vtable `0x00679A48`: open `+0x120` = `0x005C8CE0`, close
/// `+0x118` = `0x005C93A0`, per-frame update `+0xF4` = `0x005C94F0`, draw `+0x9C` =
/// `0x005C9830`.
///
/// Same family as the tavern and the trading office, and the same page shape: the
/// window opens on an empty page `-1` (`0x005C94AC` writes it) and both the update and
/// the draw method load [UIChurchWindowPtr::get_selected_page] with a 6-byte
/// `mov eax,[reg+0x1D30]` before dispatching through a jump table - which is what makes
/// the details-page detour possible, exactly as in `mod-tavern-details`:
///
/// | phase | page load | continuation | bound |
/// |-|-|-|-|
/// | update (`this` in edi) | `0x005C9542` | `0x005C9548` | `cmp eax,3 / ja 0x005C956D` |
/// | draw (`this` in esi) | `0x005C98A5` | `0x005C98AB` | `cmp eax,5 / ja 0x005C98E5` |
///
/// The compares are unsigned, so `-1` misses every case and the window draws nothing but
/// its frame - the empty page a details mod fills in.
///
/// The page switcher is `0x005C9D90` (it writes the field at `0x005C9DD1`).
#[derive(Clone, Debug, Copy)]
pub struct UIChurchWindowPtr {
    pub address: u32,
}

impl Default for UIChurchWindowPtr {
    fn default() -> Self {
        Self::new()
    }
}

impl UIChurchWindowPtr {
    pub const VTABLE_OFFSET: u32 = 0x279A48;

    pub fn new() -> Self {
        Self {
            address: unsafe { *STATIC_UI_CHURCH_WINDOW_PTR_ADDRESS },
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

    /// The town the church belongs to, read by the window's own tick.
    pub unsafe fn get_town_index(&self) -> i32 {
        self.get(0x1d38)
    }

    /// The church's page, `-1` being the empty page the window starts on. Named
    /// "mode" by `mod-fix-church-anim-crash`, which reads the same field.
    pub unsafe fn get_selected_page(&self) -> i32 {
        self.get(0x1d30)
    }
}

impl P3Pointer for UIChurchWindowPtr {
    fn get_address(&self) -> u32 {
        self.address
    }
}
