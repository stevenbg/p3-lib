use std::{
    arch::global_asm,
    ffi::c_void,
    mem,
    sync::atomic::{AtomicPtr, Ordering},
};

use hooklet::windows::x86::{deploy_rel32_raw, hook_function_pointer, FunctionPointerHook, X86Rel32Type};
use log::error;
use p3_api::ui::ui_trading_office_window::UITradingOfficeWindowPtr;

/// The window class family: `+0x9C` is the draw method, `+0xF4` the per-frame update,
/// `+0x120` open. Both the draw and the update method load the selected page into
/// eax with a 6-byte `mov eax, [esi+0xecc4]` and dispatch through a jump table, so
/// both can be detoured the way mod-shipyard-details and mod-town-hall-details
/// detour their windows: the detour returns the same page value in eax.
///
/// The draw method (`0x005D95A0`) is where the page is rendered.
const DRAW_SELECTED_PAGE_PATCH_ADDRESS: u32 = 0x005D9674;
static DRAW_SELECTED_PAGE_CONTINUATION: u32 = 0x005D967A;
/// The update method (`0x005D9500`) is where the window's area is submitted to the
/// renderer - the phase the town hall window uses for the same call, gated by its
/// day timestamp at `+0x1930`. Submitting it from inside the draw method instead -
/// once or per frame - leaves the background art torn and the text flickering.
const UPDATE_SELECTED_PAGE_PATCH_ADDRESS: u32 = 0x005D9508;
static UPDATE_SELECTED_PAGE_CONTINUATION: u32 = 0x005D950E;

const WINDOW_OPEN_POINTER_OFFSET: u32 = UITradingOfficeWindowPtr::VTABLE_OFFSET + 0x120;
static WINDOW_OPEN_HOOK: AtomicPtr<FunctionPointerHook> = AtomicPtr::new(std::ptr::null_mut());

#[no_mangle]
pub unsafe extern "C" fn start() -> u32 {
    let _ = log::set_logger(&win_dbg_logger::DEBUGGER_LOGGER);
    log::set_max_level(log::LevelFilter::Trace);

    match hook_function_pointer(WINDOW_OPEN_POINTER_OFFSET, window_open_hook as usize as u32) {
        Ok(hook) => WINDOW_OPEN_HOOK.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
        Err(_) => {
            error!("failed to hook the trading office window's open method");
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
        error!("failed to detour the trading office update function");
        return 2;
    }

    if deploy_rel32_raw(
        DRAW_SELECTED_PAGE_PATCH_ADDRESS as _,
        (&draw_selected_page_detour) as *const _ as _,
        X86Rel32Type::Jump,
    )
    .is_err()
    {
        error!("failed to detour the trading office draw function");
        return 3;
    }

    0
}

/// Prepare the drawing state for the page, the way the other details mods do it on
/// open rather than per frame.
#[no_mangle]
unsafe extern "thiscall" fn window_open_hook(window_address: u32) {
    let orig: extern "thiscall" fn(u32) = mem::transmute((*WINDOW_OPEN_HOOK.load(Ordering::SeqCst)).old_absolute);
    orig(window_address);
    crate::details::prepare_drawing_state();
}

#[no_mangle]
unsafe extern "thiscall" fn trading_office_update_hook() -> i32 {
    let window = UITradingOfficeWindowPtr::new();
    let selected_page = window.get_selected_page();
    if selected_page == -1 {
        crate::details::invalidate(window);
    }

    selected_page
}

#[no_mangle]
unsafe extern "thiscall" fn trading_office_draw_hook() -> i32 {
    let window = UITradingOfficeWindowPtr::new();
    let selected_page = window.get_selected_page();
    if selected_page == -1 {
        crate::details::draw_page(window);
    }

    selected_page
}

extern "C" {
    static update_selected_page_detour: c_void;
    static draw_selected_page_detour: c_void;
}

global_asm!("
.global {update_selected_page_detour}
{update_selected_page_detour}:
# save regs
push ecx
push edx

call {trading_office_update_hook}

# restore regs
pop edx
pop ecx

jmp [{continuation}]
",
update_selected_page_detour = sym update_selected_page_detour,
trading_office_update_hook = sym trading_office_update_hook,
continuation = sym UPDATE_SELECTED_PAGE_CONTINUATION);

global_asm!("
.global {draw_selected_page_detour}
{draw_selected_page_detour}:
# save regs
push ecx
push edx

call {trading_office_draw_hook}

# restore regs
pop edx
pop ecx

jmp [{continuation}]
",
draw_selected_page_detour = sym draw_selected_page_detour,
trading_office_draw_hook = sym trading_office_draw_hook,
continuation = sym DRAW_SELECTED_PAGE_CONTINUATION);
