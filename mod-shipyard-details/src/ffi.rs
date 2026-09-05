//! Fills the shipyard window's empty starting page (selected page `-1`) with the yard's
//! staffing, markup and experience, and the quality levels per ship type.
//!
//! The draw method (`0x005F42B0`) loads the page with `mov eax,[esi+0xC7C]` at
//! `0x005F4320`, the update method (`0x005F4220`) with the same load at `0x005F4223`; both
//! are detoured through `p3_page`. The page has one view and no keys.

use log::{error, info};
use p3_api::ui::ui_shipyard_window::UIShipyardWindowPtr;

p3_page::details_page_detours! {
    window: UIShipyardWindowPtr,
    draw: { patch: 0x005F4320, original: [0x8b, 0x86, 0x7c, 0x0c, 0x00, 0x00], base: "esi" },
    update: { patch: 0x005F4223, original: [0x8b, 0x86, 0x7c, 0x0c, 0x00, 0x00], base: "esi" },
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
        error!("shipyard details: page detours not installed (step {step})");
        return step;
    }
    info!("shipyard details loaded");
    0
}

unsafe fn on_open(_window: UIShipyardWindowPtr) {
    crate::details::prepare_drawing_state();
}

unsafe fn on_update(window: UIShipyardWindowPtr, page: i32) {
    if page == -1 {
        crate::details::invalidate(window);
    }
}

unsafe fn on_draw(window: UIShipyardWindowPtr, page: i32) {
    if page == -1 {
        crate::details::draw_page(window);
    }
}
