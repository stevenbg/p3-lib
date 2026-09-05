//! Fills the tavern window's empty starting page (selected page `-1`) with three views
//! switched by keys, and keeps the keys registered only while that page is on screen.
//!
//! The draw method (`0x005CDC60`) loads the page with `mov eax,[esi+0x1BF4]` at
//! `0x005CE3E0`, the update method (`0x005CD540`) with `mov eax,[edi+0x1BF4]` at
//! `0x005CD54D`; both are detoured through `p3_ui`. On top of that the page hooks the
//! window's close and event slots and the page switcher, for the key registration and the
//! captains table's header clicks.

use std::{
    arch::global_asm,
    ffi::c_void,
    mem,
    sync::atomic::{AtomicPtr, AtomicU32, Ordering},
};

use hooklet::windows::x86::{deploy_rel32_raw, hook_function_pointer, FunctionPointerHook, X86Rel32Type};
use log::{error, info, warn};
use p3_api::{
    hotkeys::{HotkeysApi, MOD_ALT},
    ui::{
        page_window::{SLOT_CLOSE, SLOT_EVENT},
        ui_tavern_window::UITavernWindowPtr,
    },
};
use p3_ui::page::prepare_drawing_state;

p3_ui::details_page_detours! {
    window: UITavernWindowPtr,
    draw: { patch: 0x005CE3E0, original: [0x8b, 0x86, 0xf4, 0x1b, 0x00, 0x00], base: "esi" },
    update: { patch: 0x005CD54D, original: [0x8b, 0x87, 0xf4, 0x1b, 0x00, 0x00], base: "edi" },
    on_open: on_open,
    on_update: on_update,
    on_draw: on_draw,
}

const WINDOW_CLOSE_POINTER_OFFSET: u32 = UITavernWindowPtr::VTABLE_OFFSET + SLOT_CLOSE as u32;
static WINDOW_CLOSE_HOOK: AtomicPtr<FunctionPointerHook> = AtomicPtr::new(std::ptr::null_mut());
/// The widget event handler, `thiscall(point*, type)`, `ret 8`: the root container's input
/// dispatcher calls it on the topmost child under the cursor with the screen point. Types
/// seen in the dispatcher: 1 and 2 (button down/up), -2 (left click - the ship overview's
/// row click, play-verified here as the header click), -3.
const WINDOW_EVENT_POINTER_OFFSET: u32 = UITavernWindowPtr::VTABLE_OFFSET + SLOT_EVENT as u32;
static WINDOW_EVENT_HOOK: AtomicPtr<FunctionPointerHook> = AtomicPtr::new(std::ptr::null_mut());
const EVENT_LEFT_CLICK: i32 = -2;

/// The tavern's page switcher `set_page(page)` (`0x005CED00`): the single method
/// behind every tab click and most in-page jumps (play-verified for the hotkey
/// registry design; the eleven direct field writes all happen on pages this page
/// group is not registered on, and the -1 details page is only ever LEFT through
/// set_page). Detoured at entry: the stolen prologue is `push -1; push 0x00666C8C`
/// (7 bytes), continuation `0x005CED07`.
const SET_PAGE_PATCH_ADDRESS: u32 = 0x005CED00;
static SET_PAGE_CONTINUATION: u32 = 0x005CED07;

/// The shared hotkey registry; null = unavailable, the page keys are inert.
static HOTKEYS: AtomicPtr<HotkeysApi> = AtomicPtr::new(std::ptr::null_mut());
const OWNER: &std::ffi::CStr = c"tavern-details page";

/// The details page's keys: 1 crew (alt: every town), 2 missions (alt: every town),
/// 3 my captains (alt: with the skill caps). Registered while the tavern window shows its
/// -1 page, unregistered the moment a tab is clicked or the window closes.
const PAGE_KEYS: [(u32, u32); 6] = [
    (crate::details::PAGE_KEY_CREW, 0),
    (crate::details::PAGE_KEY_CREW, MOD_ALT),
    (crate::details::PAGE_KEY_MISSIONS, 0),
    (crate::details::PAGE_KEY_MISSIONS, MOD_ALT),
    (crate::details::PAGE_KEY_CAPTAINS, 0),
    (crate::details::PAGE_KEY_CAPTAINS, MOD_ALT),
];
static PAGE_HANDLES: [AtomicU32; 6] = [const { AtomicU32::new(0) }; 6];

unsafe fn register_page_keys() {
    let Some(api) = HOTKEYS.load(Ordering::SeqCst).as_ref() else { return };
    for (i, &(vk, mods)) in PAGE_KEYS.iter().enumerate() {
        let stale = PAGE_HANDLES[i].swap(0, Ordering::SeqCst);
        if stale != 0 {
            api.unregister(stale);
        }
        PAGE_HANDLES[i].store(api.register(OWNER, vk, mods, crate::details::page_hotkeys), Ordering::SeqCst);
    }
}

unsafe fn unregister_page_keys() {
    let Some(api) = HOTKEYS.load(Ordering::SeqCst).as_ref() else { return };
    for handle in &PAGE_HANDLES {
        let handle = handle.swap(0, Ordering::SeqCst);
        if handle != 0 {
            api.unregister(handle);
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn start() -> u32 {
    let _ = log::set_logger(&win_dbg_logger::DEBUGGER_LOGGER);
    // Not Trace: the page calls p3-api lookups every frame, and their trace! lines
    // would flood the debug log.
    log::set_max_level(log::LevelFilter::Info);

    if let Err(step) = install_page_detours() {
        error!("tavern details: page detours not installed (step {step})");
        return step;
    }
    match hook_function_pointer(WINDOW_CLOSE_POINTER_OFFSET, window_close_hook as *const () as usize as u32) {
        Ok(hook) => WINDOW_CLOSE_HOOK.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
        Err(_) => {
            error!("failed to hook the tavern window's close method");
            return 5;
        }
    }
    if deploy_rel32_raw(SET_PAGE_PATCH_ADDRESS as _, (&set_page_detour) as *const _ as _, X86Rel32Type::Jump).is_err() {
        error!("failed to detour the tavern set_page at {SET_PAGE_PATCH_ADDRESS:#010x}");
        return 6;
    }
    match hook_function_pointer(WINDOW_EVENT_POINTER_OFFSET, window_event_hook as *const () as usize as u32) {
        Ok(hook) => WINDOW_EVENT_HOOK.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
        Err(_) => {
            error!("failed to hook the tavern window's event handler");
            return 7;
        }
    }
    match HotkeysApi::bind() {
        Ok(api) => HOTKEYS.store(Box::into_raw(Box::new(api)), Ordering::SeqCst),
        Err(reason) => warn!("hotkeys registry unavailable ({reason}) - the page keys are inert"),
    }
    info!("tavern details loaded");
    0
}

/// The window always opens on the -1 details page (play-verified, 10/10 opens), so the
/// page keys are armed here as well as on a switch to it.
unsafe fn on_open(_window: UITavernWindowPtr) {
    prepare_drawing_state();
    register_page_keys();
}

/// Per frame: the captains view's scrollbar follows the page, and the page's area is
/// submitted for redraw.
unsafe fn on_update(window: UITavernWindowPtr, page: i32) {
    crate::my_captains::update(window, page == -1 && crate::details::captains_view_selected());
    if page == -1 {
        crate::details::invalidate(window);
    }
}

unsafe fn on_draw(window: UITavernWindowPtr, page: i32) {
    if page == -1 {
        crate::details::draw_page(window);
    }
}

/// Close fires on every leave path (play-verified) and is the backstop for the one
/// page transition set_page cannot report: none - but a leaked registration would
/// otherwise survive until the next open, so clear here regardless.
#[no_mangle]
unsafe extern "thiscall" fn window_close_hook(window_address: u32) {
    let orig: extern "thiscall" fn(u32) = mem::transmute((*WINDOW_CLOSE_HOOK.load(Ordering::SeqCst)).old_absolute);
    orig(window_address);
    unregister_page_keys();
    crate::my_captains::detach();
}

/// Mouse events on the tavern window. A left click on the -1 page while the captains view
/// is up goes to the table's header hit test; everything goes on to the game as well.
#[no_mangle]
unsafe extern "thiscall" fn window_event_hook(window_address: u32, point: *const [i32; 2], event: i32) {
    let orig: extern "thiscall" fn(u32, *const [i32; 2], i32) = mem::transmute((*WINDOW_EVENT_HOOK.load(Ordering::SeqCst)).old_absolute);
    orig(window_address, point, event);
    if event != EVENT_LEFT_CLICK || point.is_null() {
        return;
    }
    let window = UITavernWindowPtr { address: window_address };
    if window.get_selected_page() == -1 && crate::details::captains_view_selected() {
        let [x, y] = *point;
        crate::my_captains::on_click(window, x, y);
    }
}

/// Called from the set_page asm stub with the page being switched to: entering the
/// details page (-1) arms the keys, leaving it disarms them. set_page is called
/// redundantly with the current page, which the handle-swapping registration
/// tolerates.
#[no_mangle]
unsafe extern "C" fn on_set_page(page: i32) {
    if page == -1 {
        register_page_keys();
    } else {
        unregister_page_keys();
    }
}

extern "C" {
    static set_page_detour: c_void;
}

// set_page(this=ecx, page at [esp+4]): report the transition, preserve ecx/edx,
// re-execute the stolen SEH prologue, continue. The same recipe the lifecycle
// probe ran in play.
global_asm!("
.global {set_page_detour}
{set_page_detour}:
push ecx
push edx
mov eax, dword ptr [esp + 12]
push eax
call {on_set_page}
add esp, 4
pop edx
pop ecx
push -1
push 0x00666C8C
jmp [{continuation}]
",
set_page_detour = sym set_page_detour,
on_set_page = sym on_set_page,
continuation = sym SET_PAGE_CONTINUATION);
