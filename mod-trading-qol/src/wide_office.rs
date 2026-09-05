//! A bigger trading office window, so the administrator page can hold one more column and
//! more rows. The mechanics - sizing the window before the game lays it out, enlarging the
//! shared building backdrop behind it and keeping its picture, veil and frame right - are
//! `p3_ui::enlarge`; this module only says how big.

use p3_api::ui::ui_trading_office_window::UITradingOfficeWindowPtr;
use p3_ui::enlarge;

/// The size the office window opens at (the game's is 425 x 510).
pub const WIDTH: i32 = 490;
pub const HEIGHT: i32 = 560;

/// From `start()`: the backdrop hooks the enlargement needs. `Err` carries the failed step.
pub unsafe fn install() -> Result<(), u32> {
    enlarge::install_backdrop_hooks()
}

/// Before the game's open method.
pub unsafe fn before_open(window: &UITradingOfficeWindowPtr) {
    enlarge::before_open(window, WIDTH, HEIGHT);
}

/// After the game's open method.
pub unsafe fn after_open() {
    enlarge::after_open(WIDTH, HEIGHT);
}

/// From the window's close hook.
pub unsafe fn on_close() {
    enlarge::on_close();
}
