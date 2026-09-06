//! The **main scene**: the root widget container of the in-game screen, kept in the static
//! below (constructed by `0x00422FC0`, stored at `0x00463ACE`). The scrollmap and the
//! local-map scene are its children, as are the side panel's buttons - the ship list, the
//! Options cogs and the rest - so it is the object whose input slots see every click that no
//! window took: its key slot `+0x1C` is the keyboard dispatcher `0x004247A0` (ESC, Tab, the
//! speed keys), its right-button-up slot `+0x148` is `0x004298F0`. Other pages of the book
//! refer to the same object as the notification manager, since the ticker hangs off it too.

use crate::data::p3_ptr::P3Pointer;

pub const MAIN_SCENE_PTR_ADDRESS: *const u32 = 0x006CBB40 as _;

/// The cursor in game-resolution coordinates, written by the `WM_MOUSEMOVE` handler
/// `0x004C0D30` (client point scaled by `[0x006DAB4C]` / `[0x006DAB44]`). These are the
/// coordinates the mouse slots receive.
pub const CURSOR_X_ADDRESS: u32 = 0x006DAB50;
pub const CURSOR_Y_ADDRESS: u32 = 0x006DAB48;

#[derive(Clone, Debug)]
pub struct UIMainScenePtr {
    pub address: u32,
}

impl UIMainScenePtr {
    /// Module-relative offset of the vtable `0x0066C8A0` (89 slots), for
    /// `hook_function_pointer`.
    pub const VTABLE_OFFSET: u32 = 0x0026C8A0;
    /// `thiscall(flags, x, y)`, `ret 0xC`, reached from the `WM_RBUTTONUP` handler
    /// `0x004C0CE0` through the window stack (`0x004B93B0`) with `x` / `y` from
    /// [CURSOR_X_ADDRESS] / [CURSOR_Y_ADDRESS]. The scene's own `0x004298F0` closes the
    /// topmost open window when one is open (`0x00462310`, `0x00462230`) and hands the
    /// event to the container base `0x004B6400`, which passes `-3` to the pressed and the
    /// focused child - not to the child under the cursor.
    pub const SLOT_RIGHT_BUTTON_UP: u32 = 0x148;
    pub const RIGHT_BUTTON_UP_ADDRESS: u32 = 0x004298F0;
    /// The **Options** button (the cogs under the minimap), a `CViperButton` embedded at
    /// `+0x16F0`, constructed at `0x00423144` and registered with the scene at `0x00425BB7`.
    /// The per-frame update polls it at `0x00423A73` and opens the game menu when it was
    /// clicked or when [Self::MENU_REQUEST_FLAG_OFFSET] is set.
    pub const OPTIONS_BUTTON_OFFSET: u32 = 0x16F0;
    /// The byte ESC sets (`0x004247F9`) when no window is open: "open the game menu on the
    /// next update".
    pub const MENU_REQUEST_FLAG_OFFSET: u32 = 0x2636;

    /// The scene, or `None` before the game has built it.
    pub fn new() -> Option<Self> {
        let address = unsafe { *MAIN_SCENE_PTR_ADDRESS };
        if (0x0001_0000..0x7fff_0000).contains(&address) {
            Some(Self { address })
        } else {
            None
        }
    }

    /// The Options button widget.
    pub fn options_button(&self) -> u32 {
        self.address + Self::OPTIONS_BUTTON_OFFSET
    }

    /// What ESC does with no window open: ask the next update to open the game menu.
    pub unsafe fn request_menu(&self) {
        self.set(Self::MENU_REQUEST_FLAG_OFFSET, &1u8)
    }
}

impl P3Pointer for UIMainScenePtr {
    fn get_address(&self) -> u32 {
        self.address
    }
}
