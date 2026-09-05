//! Fills the town hall window's empty starting page (selected page `-1`) with the
//! Hanse-wide goods table, and adds the mission's figures to the alderman's office
//! (page 7).
//!
//! The draw method loads the page with `mov eax,[esi+0x18E4]` at `0x005E09AC`, the update
//! method (`0x005E0850`) with the same load at `0x005E085B`; both are detoured through
//! `p3_page`. Page 7 is a page the game draws itself, so switching to it re-applies the
//! drawing state: the side panel's call to `set_selected_page` is hooked for that.

use std::{
    mem,
    sync::atomic::{AtomicPtr, Ordering},
};

use hooklet::windows::x86::{hook_call_rel32, CallRel32Hook};
use log::{error, info};
use p3_api::ui::ui_town_hall_window::UITownHallWindowPtr;
use p3_page::{page::prepare_drawing_state, Page};

use crate::pages::{aldermans_office, details};

p3_page::details_page_detours! {
    window: UITownHallWindowPtr,
    draw: { patch: 0x005E09AC, original: [0x8b, 0x86, 0xe4, 0x18, 0x00, 0x00], base: "esi" },
    update: { patch: 0x005E085B, original: [0x8b, 0x86, 0xe4, 0x18, 0x00, 0x00], base: "esi" },
    on_open: on_open,
    on_update: on_update,
    on_draw: on_draw,
}

/// The alderman's office page, which the game draws itself and this mod adds to.
const ALDERMANS_OFFICE_PAGE: i32 = 7;

/// The side panel's call to the window's `set_selected_page`, module-relative.
const SIDEPANEL_SET_SELECTED_PAGE_CALL_OFFSET: u32 = 0x1A94BC;
static SET_SELECTED_PAGE_HOOK: AtomicPtr<CallRel32Hook> = AtomicPtr::new(std::ptr::null_mut());

#[no_mangle]
pub unsafe extern "C" fn start() -> u32 {
    let _ = log::set_logger(&win_dbg_logger::DEBUGGER_LOGGER);
    log::set_max_level(log::LevelFilter::Info);

    if let Err(step) = install_page_detours() {
        error!("town hall details: page detours not installed (step {step})");
        return step;
    }
    match hook_call_rel32(SIDEPANEL_SET_SELECTED_PAGE_CALL_OFFSET, set_selected_page_hook as *const () as usize as u32) {
        Ok(hook) => SET_SELECTED_PAGE_HOOK.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
        Err(_) => {
            error!("failed to hook the side panel's set_selected_page call");
            return 5;
        }
    }
    info!("town hall details loaded");
    0
}

unsafe fn on_open(_window: UITownHallWindowPtr) {
    prepare_drawing_state();
}

unsafe fn on_update(window: UITownHallWindowPtr, page: i32) {
    if page == -1 || page == ALDERMANS_OFFICE_PAGE {
        Page::new(&window, 0).invalidate();
    }
}

unsafe fn on_draw(window: UITownHallWindowPtr, page: i32) {
    if page == -1 {
        details::draw_page(window);
    } else if page == ALDERMANS_OFFICE_PAGE {
        aldermans_office::draw_page(window);
    }
}

/// The game's own drawing of the alderman's office sets up its clipping, so the drawing
/// state is re-applied whenever that page is selected.
#[no_mangle]
unsafe extern "thiscall" fn set_selected_page_hook(window_address: u32, page: u32) {
    let orig: extern "thiscall" fn(u32, u32) = mem::transmute((*SET_SELECTED_PAGE_HOOK.load(Ordering::SeqCst)).old_absolute);
    orig(window_address, page);
    if page as i32 == ALDERMANS_OFFICE_PAGE {
        prepare_drawing_state();
    }
}
