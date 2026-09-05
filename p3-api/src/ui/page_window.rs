//! The building windows whose spare page a details mod fills, behind one trait: the
//! geometry a page is laid out in, the selected-page field the page dispatch reads, and
//! the window's own text-layout object for rich text.
//!
//! The windows are one class family. Their frame is at `+0x14`/`+0x18` (x, y) and
//! `+0x2C`/`+0x30` (width, height); the vtable slots a page mod hooks are
//! [SLOT_OPEN], [SLOT_CLOSE] and [SLOT_EVENT]. Where the selected page lives differs per
//! window, as does the layout object - the constants in [crate::ui::rich_text].

use crate::ui::{
    rich_text::{CHURCH_WINDOW_LAYOUT_OFFSET, TAVERN_WINDOW_LAYOUT_OFFSET, TOWN_HALL_WINDOW_LAYOUT_OFFSET},
    ui_church_window::UIChurchWindowPtr,
    ui_tavern_window::UITavernWindowPtr,
    ui_town_hall_window::UITownHallWindowPtr,
    ui_trading_office_window::UITradingOfficeWindowPtr,
};
pub use crate::ui::widget::{SLOT_CLOSE, SLOT_EVENT, SLOT_OPEN};

pub trait PageWindow: Copy {
    /// The vtable's module-relative offset, for `hook_function_pointer` on its slots.
    const VTABLE_OFFSET: u32;
    /// The window's text-layout object relative to the window, for windows that own one;
    /// rich text (inline coin, load and barrel symbols) needs it.
    const LAYOUT_OFFSET: Option<u32>;

    /// The window the game is dispatching on - `this` in a hooked method - which is not
    /// always the object behind the window's global pointer.
    fn from_address(address: u32) -> Self;
    fn address(&self) -> u32;
    fn x(&self) -> i32;
    fn y(&self) -> i32;
    fn width(&self) -> i32;
    fn height(&self) -> i32;
    /// The selected page; `-1` is the empty page the window opens on, which the details
    /// mods fill.
    unsafe fn selected_page(&self) -> i32;

    fn layout(&self) -> Option<u32> {
        Self::LAYOUT_OFFSET.map(|offset| self.address() + offset)
    }
}

macro_rules! page_window {
    ($window:ty, $vtable:expr, $layout:expr) => {
        impl PageWindow for $window {
            const VTABLE_OFFSET: u32 = $vtable;
            const LAYOUT_OFFSET: Option<u32> = $layout;

            fn from_address(address: u32) -> Self {
                Self { address }
            }
            fn address(&self) -> u32 {
                self.address
            }
            fn x(&self) -> i32 {
                self.get_x()
            }
            fn y(&self) -> i32 {
                self.get_y()
            }
            fn width(&self) -> i32 {
                self.get_width()
            }
            fn height(&self) -> i32 {
                self.get_height()
            }
            unsafe fn selected_page(&self) -> i32 {
                self.get_selected_page()
            }
        }
    };
}

page_window!(UIChurchWindowPtr, UIChurchWindowPtr::VTABLE_OFFSET, Some(CHURCH_WINDOW_LAYOUT_OFFSET));
page_window!(UITavernWindowPtr, UITavernWindowPtr::VTABLE_OFFSET, Some(TAVERN_WINDOW_LAYOUT_OFFSET));
page_window!(UITradingOfficeWindowPtr, UITradingOfficeWindowPtr::VTABLE_OFFSET, None);
page_window!(UITownHallWindowPtr, UITownHallWindowPtr::VTABLE_OFFSET, Some(TOWN_HALL_WINDOW_LAYOUT_OFFSET));
