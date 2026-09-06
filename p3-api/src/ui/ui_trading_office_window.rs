use std::mem;

use crate::data::p3_ptr::P3Pointer;

use super::number_widget::{self, NumberWidget};

pub const STATIC_UI_TRADING_OFFICE_WINDOW_PTR_ADDRESS: *const u32 = 0x006E557C as _;

/// The administrator view ("Trading Office" side button) of the pages 0-6.
pub const ADMINISTRATOR_PAGE: i32 = 4;
/// The administrator view's ware rows: 20 rows in the display order of
/// [`WARE_DISPLAY_ORDER`], the first at `window.y + FIRST_ROW_Y`, then every `ROW_PITCH`
/// pixels (the open method's placement loop, `0x005D8E2B` / `0x005D9237`). The row widgets
/// are placed with their top-left corner on the row's y.
pub const ROW_COUNT: usize = 20;
pub const FIRST_ROW_Y: i32 = 0x46;
pub const ROW_PITCH: i32 = 0x14;
/// The ware index shown on each row (identity in the executable, sorted at runtime by the
/// localized ware name).
pub const WARE_DISPLAY_ORDER: *const u8 = 0x0069_8538 as _;
/// The lock checkbox of each row is two widgets: a round `CViperButton` (template 0) in
/// the array at `+0x861C` (stride `0xE8`) and, registered after it so it draws on top, the
/// `C2DAnimation` checkmark in the array at `+0xDBC0` (stride `0xD8`) at `(button.x - 4,
/// button.y)`. The page update toggles the checkmark's visibility on the button's click
/// (`0x005DD434`) and enqueues operation `0x66`.
pub const LOCK_BUTTONS_OFFSET: u32 = 0x861C;
pub const LOCK_CHECKMARKS_OFFSET: u32 = 0xDBC0;
/// The X close button, a `CViperButton` clone of template 5 at `(x + w - 32 - 3, y + h - 18 - 3)`;
/// the update method closes the window when it reports a click (`0x005D955D`).
pub const CLOSE_BUTTON_OFFSET: u32 = 0xD0;
/// The margin the close button keeps from the window's bottom-right corner.
pub const CORNER_BUTTON_MARGIN: i32 = 3;
/// The administrator view's two [number box](super::number_widget) columns, 20 boxes each
/// (stride [`super::number_widget::OBJECT_SIZE`]) in row order: the amounts (in-game units,
/// populated only by the open method) and the prices.
pub const AMOUNT_WIDGETS_OFFSET: u32 = 0x9840;
/// The twenty amount `+` buttons (`0xE8` each), the widget left of the room between the
/// amount and price groups.
pub const AMOUNT_PLUS_BUTTONS_OFFSET: u32 = 0x195C;
pub const BUTTON_SIZE: u32 = 0xE8;
pub const PRICE_WIDGETS_OFFSET: u32 = 0xBA00;

/// The open method lays each row out from the right edge inwards (`0x005D8E9F`..
/// `0x005D8EF3`): the price `+` button's right edge is `width - 25` - the `-0x19` of the
/// `lea eax,[ecx+edx*1-0x19]` at `0x005D8EA9`, whose displacement byte is this site - and
/// every other row widget is placed leftwards from it by its width and a fixed gap.
pub const ROW_RIGHT_MARGIN_SITE: u32 = 0x005D8EAC;
pub const ROW_RIGHT_MARGIN_ORIGINAL: u8 = 0xE7;
/// The gap between the lock button and the price `-` button: the `sub eax,0x1F` at
/// `0x005D8EC7`, its immediate byte at this site. Moving the price group without moving
/// the lock and amount widgets means widening this by the same amount.
pub const ROW_LOCK_GAP_SITE: u32 = 0x005D8EC9;
pub const ROW_LOCK_GAP_ORIGINAL: u8 = 0x1F;
/// The gap between the amount `+` button and the lock button: the `sub eax,0x3` at
/// `0x005D8ED0`, its immediate byte at this site. The two gaps together are the room
/// between the amount and price groups; shifting the lock button is moving room from one
/// to the other.
pub const ROW_AMOUNT_GAP_SITE: u32 = 0x005D8ED2;
pub const ROW_AMOUNT_GAP_ORIGINAL: u8 = 0x03;
/// The administrator page's "max" / "min" label, drawn per row by the page draw
/// (`0x005DAC80`): the buy-arrow-visible branch pushes `[0x0069B31C]` ("max"), the
/// sell-arrow branch `[0x0069B320]` ("min"), each with the row's y and then the price `-`
/// button's x (its `+0x6C` getter) minus 3, and both fall into one cdecl
/// `call 0x004BB3E0` (render text) at this site, followed by `add esp,0xC`. Silencing the
/// label means replacing that call, not the pushes: the string slots are shared with the
/// auto-trade goods dialog (`0x00404A3E`, `0x00404A52`), and the pushes are the call's
/// arguments, cleaned by the caller.
pub const LABEL_RENDER_CALL_SITE: u32 = 0x005DAEAD;
pub const LABEL_RENDER_CALL_ORIGINAL: [u8; 5] = [0xe8, 0x2e, 0x05, 0xee, 0xff];

#[derive(Clone, Debug, Copy)]
pub struct UITradingOfficeWindowPtr {
    pub address: u32,
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

    /// The side menu view: 0 Total, ..., 4 Trading Office (administrator), ...;
    /// `-1` is the empty starting page the window opens on.
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

    /// The amount box of a row of the administrator view.
    pub fn amount_widget(&self, row: usize) -> NumberWidget {
        NumberWidget::new(self.address + AMOUNT_WIDGETS_OFFSET + row as u32 * number_widget::OBJECT_SIZE)
    }

    /// The price box of a row of the administrator view.
    pub fn price_widget(&self, row: usize) -> NumberWidget {
        NumberWidget::new(self.address + PRICE_WIDGETS_OFFSET + row as u32 * number_widget::OBJECT_SIZE)
    }

    /// The row whose price box sits at `address`, if it is one of this window's.
    pub fn price_widget_row(&self, address: u32) -> Option<usize> {
        let first = self.address.checked_add(PRICE_WIDGETS_OFFSET)?;
        let offset = address.checked_sub(first)?;
        if offset % number_widget::OBJECT_SIZE != 0 {
            return None;
        }
        let row = (offset / number_widget::OBJECT_SIZE) as usize;
        (row < ROW_COUNT).then_some(row)
    }
}

impl P3Pointer for UITradingOfficeWindowPtr {
    fn get_address(&self) -> u32 {
        self.address
    }
}
