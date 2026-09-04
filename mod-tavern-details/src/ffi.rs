use std::{
    arch::global_asm,
    ffi::c_void,
    mem,
    sync::atomic::{AtomicPtr, Ordering},
};

use hooklet::windows::x86::{deploy_rel32_raw, hook_function_pointer, FunctionPointerHook, X86Rel32Type};
use log::{error, warn};
use p3_api::{
    hotkeys::{HotkeysApi, MOD_ALT},
    ui::ui_tavern_window::UITavernWindowPtr,
};
use std::sync::atomic::AtomicU32;


/// The window class family: `+0x9C` is the draw method, `+0xF4` the per-frame update,
/// `+0x120` open. Both the draw and the update method load the selected page into eax
/// with a 6-byte `mov eax, [reg+0x1bf4]` and dispatch through a jump table, so both
/// can be detoured the way mod-trading-office-details and mod-town-hall-details
/// detour their windows: the detour returns the same page value in eax.
///
/// The draw method (`0x005CDC60`) is where the page is rendered.
const DRAW_SELECTED_PAGE_PATCH_ADDRESS: u32 = 0x005CE3E0;
static DRAW_SELECTED_PAGE_CONTINUATION: u32 = 0x005CE3E6;
/// The update method (`0x005CD540`) is where the window's area is submitted to the
/// renderer - the phase the town hall window uses for the same call. Submitting it
/// from inside the draw method instead - once or per frame - leaves the background art
/// torn and the text flickering.
const UPDATE_SELECTED_PAGE_PATCH_ADDRESS: u32 = 0x005CD54D;
static UPDATE_SELECTED_PAGE_CONTINUATION: u32 = 0x005CD553;

const WINDOW_OPEN_POINTER_OFFSET: u32 = UITavernWindowPtr::VTABLE_OFFSET + 0x120;
static WINDOW_OPEN_HOOK: AtomicPtr<FunctionPointerHook> = AtomicPtr::new(std::ptr::null_mut());
const WINDOW_CLOSE_POINTER_OFFSET: u32 = UITavernWindowPtr::VTABLE_OFFSET + 0x118;
static WINDOW_CLOSE_HOOK: AtomicPtr<FunctionPointerHook> = AtomicPtr::new(std::ptr::null_mut());
/// The widget event handler, `thiscall(point*, type)`, `ret 8`: the root container's input
/// dispatcher calls it on the topmost child under the cursor with the screen point. Types
/// seen in the dispatcher: 1 and 2 (button down/up), -2 (left click - the ship overview's
/// row click, play-verified here as the header click), -3.
const WINDOW_EVENT_POINTER_OFFSET: u32 = UITavernWindowPtr::VTABLE_OFFSET + 0x18;
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


    match hook_function_pointer(WINDOW_OPEN_POINTER_OFFSET, window_open_hook as usize as u32) {
        Ok(hook) => WINDOW_OPEN_HOOK.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
        Err(_) => {
            error!("failed to hook the tavern window's open method");
            return 1;
        }
    }

    if deploy_rel32_raw(
        UPDATE_SELECTED_PAGE_PATCH_ADDRESS as _,
        (&update_selected_page_detour) as *const _ as _,
        X86Rel32Type::Jump,
    )
    .is_err()
    {
        error!("failed to detour the tavern update function");
        return 2;
    }

    if deploy_rel32_raw(
        DRAW_SELECTED_PAGE_PATCH_ADDRESS as _,
        (&draw_selected_page_detour) as *const _ as _,
        X86Rel32Type::Jump,
    )
    .is_err()
    {
        error!("failed to detour the tavern draw function");
        return 3;
    }

    if let Err(what) = crate::letter_popups::install() {
        error!("failed to hook {what}");
        return 4;
    }

    match hook_function_pointer(WINDOW_CLOSE_POINTER_OFFSET, window_close_hook as usize as u32) {
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
    match hook_function_pointer(WINDOW_EVENT_POINTER_OFFSET, window_event_hook as usize as u32) {
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

    0
}

/// Prepare the drawing state for the page, the way the other details mods do it on
/// open rather than per frame - and arm the page keys: the window always opens on
/// the -1 details page (play-verified, 10/10 opens).
#[no_mangle]
unsafe extern "thiscall" fn window_open_hook(window_address: u32) {
    let orig: extern "thiscall" fn(u32) = mem::transmute((*WINDOW_OPEN_HOOK.load(Ordering::SeqCst)).old_absolute);
    orig(window_address);
    crate::details::prepare_drawing_state();
    register_page_keys();
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
    let window = UITavernWindowPtr::new();
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

#[no_mangle]
unsafe extern "thiscall" fn tavern_update_hook() -> i32 {
    let window = UITavernWindowPtr::new();
    let selected_page = window.get_selected_page();
    crate::my_captains::update(window, selected_page == -1 && crate::details::captains_view_selected());
    if selected_page == -1 {
        crate::details::invalidate(window);
    }

    selected_page
}

#[no_mangle]
unsafe extern "thiscall" fn tavern_draw_hook() -> i32 {
    let window = UITavernWindowPtr::new();
    let selected_page = window.get_selected_page();
    if selected_page == -1 {
        crate::details::draw_page(window);
    }

    selected_page
}

extern "C" {
    static update_selected_page_detour: c_void;
    static draw_selected_page_detour: c_void;
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

global_asm!("
.global {update_selected_page_detour}
{update_selected_page_detour}:
# save regs
push ecx
push edx

call {tavern_update_hook}

# restore regs
pop edx
pop ecx

jmp [{continuation}]
",
update_selected_page_detour = sym update_selected_page_detour,
tavern_update_hook = sym tavern_update_hook,
continuation = sym UPDATE_SELECTED_PAGE_CONTINUATION);

global_asm!("
.global {draw_selected_page_detour}
{draw_selected_page_detour}:
# save regs
push ecx
push edx

call {tavern_draw_hook}

# restore regs
pop edx
pop ecx

jmp [{continuation}]
",
draw_selected_page_detour = sym draw_selected_page_detour,
tavern_draw_hook = sym tavern_draw_hook,
continuation = sym DRAW_SELECTED_PAGE_CONTINUATION);
