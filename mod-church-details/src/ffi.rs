//! Fills the church window's empty starting page (selected page `-1`, the one the window
//! opens on) with what the church's three actions in this town are worth.
//!
//! `p3_api`'s [`UIChurchWindowPtr`](p3_api::ui::ui_church_window::UIChurchWindowPtr)
//! documents the vtable and the two page-dispatch sites detoured here: the draw method
//! (`0x005C9830`) loads the page with `mov eax,[esi+0x1D30]` at `0x005C98A5`, the update
//! method (`0x005C94F0`) with `mov eax,[edi+0x1D30]` at `0x005C9542`. The page has one
//! view and no keys.

use log::{error, info};
use p3_api::ui::ui_church_window::UIChurchWindowPtr;
use p3_page::page::prepare_drawing_state;

p3_page::details_page_detours! {
    window: UIChurchWindowPtr,
    draw: { patch: 0x005C98A5, original: [0x8b, 0x86, 0x30, 0x1d, 0x00, 0x00], base: "esi" },
    update: { patch: 0x005C9542, original: [0x8b, 0x87, 0x30, 0x1d, 0x00, 0x00], base: "edi" },
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
        error!("church details: page detours not installed (step {step})");
        return step;
    }
    info!("church details loaded");
    0
}

unsafe fn on_open(_window: UIChurchWindowPtr) {
    prepare_drawing_state();
}

unsafe fn on_update(window: UIChurchWindowPtr, page: i32) {
    if page == -1 {
        crate::details::invalidate(window);
    }
}

unsafe fn on_draw(window: UIChurchWindowPtr, page: i32) {
    if page == -1 {
        crate::details::draw_page(window);
    }
}
