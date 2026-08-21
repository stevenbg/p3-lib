use std::{
    arch::global_asm,
    ffi::c_void,
    mem,
    sync::atomic::{AtomicPtr, Ordering},
};

use hooklet::windows::x86::{deploy_rel32_raw, hook_function_pointer, FunctionPointerHook, X86Rel32Type};
use log::error;
use p3_api::ui::ui_tavern_window::UITavernWindowPtr;

/// Sets the PEB BeingDebugged flag so IsDebuggerPresent() returns true, unlocking the
/// gated win_dbg_logger used by every mod. No real debugger is attached, so log output
/// still reaches DebugView via OutputDebugString. It lives in this mod (rather than in
/// one of the automation mods) so it is easy to find and remove later; it used to live
/// in mod-ui-tweaks, which this mod absorbed.
#[cfg(target_arch = "x86")]
unsafe fn fake_being_debugged() {
    let peb: *mut u8;
    std::arch::asm!("mov {}, fs:[0x30]", out(reg) peb);
    // PEB + 0x02 = BeingDebugged (u8).
    *peb.add(2) = 1;
}

#[cfg(not(target_arch = "x86"))]
unsafe fn fake_being_debugged() {}

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

#[no_mangle]
pub unsafe extern "C" fn start() -> u32 {
    let _ = log::set_logger(&win_dbg_logger::DEBUGGER_LOGGER);
    // Not Trace: the page calls p3-api lookups every frame, and their trace! lines
    // would flood the debug log.
    log::set_max_level(log::LevelFilter::Info);

    fake_being_debugged();

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
unsafe extern "thiscall" fn tavern_update_hook() -> i32 {
    let window = UITavernWindowPtr::new();
    let selected_page = window.get_selected_page();
    if selected_page == -1 {
        crate::details::poll_keys();
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
}

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
