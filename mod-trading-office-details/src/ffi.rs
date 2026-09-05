//! Fills the trading office window's empty starting page (selected page `-1`) with the
//! town facts the game shows nowhere.
//!
//! The draw method (`0x005D95A0`) loads the page with `mov eax,[esi+0xECC4]` at
//! `0x005D9674`, the update method (`0x005D9500`) with the same load at `0x005D9508`; both
//! are detoured through `p3_ui`.

use log::{error, info};
use p3_api::ui::ui_trading_office_window::UITradingOfficeWindowPtr;
use p3_ui::page::prepare_drawing_state;

p3_ui::details_page_detours! {
    window: UITradingOfficeWindowPtr,
    draw: { patch: 0x005D9674, original: [0x8b, 0x86, 0xc4, 0xec, 0x00, 0x00], base: "esi" },
    update: { patch: 0x005D9508, original: [0x8b, 0x86, 0xc4, 0xec, 0x00, 0x00], base: "esi" },
    on_open: on_open,
    on_update: on_update,
    on_draw: on_draw,
}

#[no_mangle]
pub unsafe extern "C" fn start() -> u32 {
    let _ = log::set_logger(&win_dbg_logger::DEBUGGER_LOGGER);
    // Not Trace: the page calls p3-api lookups every frame, and their trace! lines would
    // flood the debug log.
    log::set_max_level(log::LevelFilter::Info);

    if let Err(step) = install_page_detours() {
        error!("trading office details: page detours not installed (step {step})");
        return step;
    }
    info!("trading office details loaded");
    0
}

unsafe fn on_open(_window: UITradingOfficeWindowPtr) {
    prepare_drawing_state();
}

unsafe fn on_update(window: UITradingOfficeWindowPtr, page: i32) {
    if page == -1 {
        crate::details::invalidate(window);
    }
}

unsafe fn on_draw(window: UITradingOfficeWindowPtr, page: i32) {
    if page == -1 {
        crate::details::draw_page(window);
    }
}
