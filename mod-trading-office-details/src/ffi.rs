//! Fills the trading office window's empty starting page (selected page `-1`) with the
//! town facts the game shows nowhere.
//!
//! The draw method (`0x005D95A0`) loads the page with `mov eax,[esi+0xECC4]` at
//! `0x005D9674`, the update method (`0x005D9500`) with the same load at `0x005D9508`; both
//! are detoured through `p3_ui`. The Total page (page 0) is a page the game draws itself,
//! so switching to it re-applies the drawing state: the side-menu controller's call to the
//! window's `set_selected_page` (`0x005D9A20`, its only caller at `0x005A4CC4`) is hooked
//! for that.

use std::{
    mem,
    sync::atomic::{AtomicPtr, Ordering},
};

use log::{error, info};
use p3_api::ui::ui_trading_office_window::UITradingOfficeWindowPtr;
use p3_ui::{
    hooklet::windows::x86::{hook_call_rel32, CallRel32Hook},
    page::prepare_drawing_state,
};

/// The game's Total page, which this mod adds the administrator line to.
const TOTAL_PAGE: i32 = 0;
/// The side-menu controller's `call 0x005D9A20` (`set_selected_page`), module-relative.
const SET_SELECTED_PAGE_CALL_OFFSET: u32 = 0x1A4CC4;
static SET_SELECTED_PAGE_HOOK: AtomicPtr<CallRel32Hook> = AtomicPtr::new(std::ptr::null_mut());

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
    match hook_call_rel32(SET_SELECTED_PAGE_CALL_OFFSET, set_selected_page_hook as *const () as usize as u32) {
        Ok(hook) => SET_SELECTED_PAGE_HOOK.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
        Err(_) => {
            error!("failed to hook the side menu's set_selected_page call");
            return 5;
        }
    }
    info!("trading office details loaded");
    0
}

unsafe fn on_open(_window: UITradingOfficeWindowPtr) {
    prepare_drawing_state();
}

unsafe fn on_update(window: UITradingOfficeWindowPtr, page: i32) {
    if page == -1 || page == TOTAL_PAGE {
        crate::details::invalidate(window);
    }
}

unsafe fn on_draw(window: UITradingOfficeWindowPtr, page: i32) {
    match page {
        -1 => crate::details::draw_page(window),
        // Drawn before the game's own page, which paints only text over it.
        TOTAL_PAGE => crate::details::draw_total_page(window),
        _ => {}
    }
}

/// The game's own drawing of the Total page sets up its clipping, so the drawing state is
/// re-applied whenever that page is selected.
#[no_mangle]
unsafe extern "thiscall" fn set_selected_page_hook(window_address: u32, page: u32) {
    let orig: extern "thiscall" fn(u32, u32) = mem::transmute((*SET_SELECTED_PAGE_HOOK.load(Ordering::SeqCst)).old_absolute);
    orig(window_address, page);
    if page as i32 == TOTAL_PAGE {
        prepare_drawing_state();
    }
}
