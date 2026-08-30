//! Fills the church window's empty starting page (selected page `-1`, the one the window
//! opens on) with what a "feeding the poor" donation to this town is worth, the way
//! `mod-tavern-details`, `mod-shipyard-details`, `mod-town-hall-details` and
//! `mod-trading-office-details` fill theirs.
//!
//! The church window is the same class family as those: `p3_api`'s
//! [`UIChurchWindowPtr`](p3_api::ui::ui_church_window::UIChurchWindowPtr) documents the
//! vtable and the two page-dispatch sites this mod detours. Both the per-frame update and
//! the draw method load the page into `eax` with a 6-byte `mov eax,[reg+0x1D30]` and
//! dispatch through a jump table bounded by an **unsigned** compare, so page `-1` misses
//! every case and the window draws nothing but its frame. The detours return the same page
//! value in `eax`, and do their own work first when the page is `-1`.
//!
//! There are no page keys: the page has one view.

use std::{
    arch::global_asm,
    ffi::c_void,
    mem,
    sync::atomic::{AtomicPtr, AtomicU32, Ordering},
};

use hooklet::windows::x86::{deploy_rel32_raw, hook_function_pointer, FunctionPointerHook, X86Rel32Type};
use log::{error, info};
use p3_api::ui::ui_church_window::UIChurchWindowPtr;

/// The draw method (`0x005C9830`) is where the page is rendered. Its page load is
/// `mov eax,[esi+0x1D30]`, 6 bytes, followed by `cmp eax,5 / ja`.
const DRAW_SELECTED_PAGE_PATCH_ADDRESS: u32 = 0x005C98A5;
static DRAW_SELECTED_PAGE_CONTINUATION: u32 = 0x005C98AB;
/// The update method (`0x005C94F0`) is where the window's area is submitted to the
/// renderer - the phase the other details mods use for the same call. Submitting it from
/// inside the draw method instead leaves the background art torn and the text flickering.
/// Its page load is `mov eax,[edi+0x1D30]`, 6 bytes, followed by `cmp eax,3 / ja`.
const UPDATE_SELECTED_PAGE_PATCH_ADDRESS: u32 = 0x005C9542;
static UPDATE_SELECTED_PAGE_CONTINUATION: u32 = 0x005C9548;

/// The 6 bytes each detour replaces, verified before anything is written so a different
/// game build fails loudly instead of corrupting code. Both load into **eax**; they differ
/// only in the base register - `esi` in the draw (ModRM `86`), `edi` in the update (`87`).
const DRAW_ORIGINAL: [u8; 6] = [0x8b, 0x86, 0x30, 0x1d, 0x00, 0x00];
const UPDATE_ORIGINAL: [u8; 6] = [0x8b, 0x87, 0x30, 0x1d, 0x00, 0x00];

const WINDOW_OPEN_POINTER_OFFSET: u32 = UIChurchWindowPtr::VTABLE_OFFSET + 0x120;
static WINDOW_OPEN_HOOK: AtomicPtr<FunctionPointerHook> = AtomicPtr::new(std::ptr::null_mut());

/// Sets the PEB BeingDebugged flag so `IsDebuggerPresent()` returns true, which is what
/// unlocks `win_dbg_logger`'s output. `mod-tavern-details` does the same, but the loader
/// walks `mods\` in directory order and `church_details` sorts before `tavern_details` -
/// so without this the load-time lines below are swallowed and the mod looks like it never
/// loaded. The flag is idempotent, so setting it twice costs nothing.
#[cfg(target_arch = "x86")]
unsafe fn fake_being_debugged() {
    let peb: *mut u8;
    std::arch::asm!("mov {}, fs:[0x30]", out(reg) peb);
    *peb.add(2) = 1;
}

#[cfg(not(target_arch = "x86"))]
unsafe fn fake_being_debugged() {}

#[no_mangle]
pub unsafe extern "C" fn start() -> u32 {
    let _ = log::set_logger(&win_dbg_logger::DEBUGGER_LOGGER);
    fake_being_debugged();
    // Not Trace: the page calls p3-api lookups every frame, and their trace! lines would
    // flood the debug log.
    log::set_max_level(log::LevelFilter::Info);

    for (address, expected) in [
        (DRAW_SELECTED_PAGE_PATCH_ADDRESS, DRAW_ORIGINAL),
        (UPDATE_SELECTED_PAGE_PATCH_ADDRESS, UPDATE_ORIGINAL),
    ] {
        let found = *(address as *const [u8; 6]);
        if found != expected {
            error!("unexpected bytes at the church page dispatch {address:#010x}: {found:02x?} - not patching");
            return 1;
        }
    }

    match hook_function_pointer(WINDOW_OPEN_POINTER_OFFSET, window_open_hook as *const () as u32) {
        Ok(hook) => WINDOW_OPEN_HOOK.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
        Err(_) => {
            error!("failed to hook the church window's open method");
            return 2;
        }
    }

    if deploy_rel32_raw(
        UPDATE_SELECTED_PAGE_PATCH_ADDRESS as _,
        (&update_selected_page_detour) as *const _ as _,
        X86Rel32Type::Jump,
    )
    .is_err()
    {
        error!("failed to detour the church update function");
        return 3;
    }

    if deploy_rel32_raw(
        DRAW_SELECTED_PAGE_PATCH_ADDRESS as _,
        (&draw_selected_page_detour) as *const _ as _,
        X86Rel32Type::Jump,
    )
    .is_err()
    {
        error!("failed to detour the church draw function");
        return 4;
    }

    info!("church details loaded: page dispatch detoured at {DRAW_SELECTED_PAGE_PATCH_ADDRESS:#010x} and {UPDATE_SELECTED_PAGE_PATCH_ADDRESS:#010x}");
    0
}

/// Prepare the drawing state for the page, the way the other details mods do it on open
/// rather than per frame.
#[no_mangle]
unsafe extern "thiscall" fn window_open_hook(window_address: u32) {
    let orig: extern "thiscall" fn(u32) = mem::transmute((*WINDOW_OPEN_HOOK.load(Ordering::SeqCst)).old_absolute);
    orig(window_address);
    // TEMPORARY: is the global the object the vtable hook fired on?
    info!(
        "church open: this={window_address:#010x} global={:#010x} page={}",
        *p3_api::ui::ui_church_window::STATIC_UI_CHURCH_WINDOW_PTR_ADDRESS,
        *((window_address + 0x1d30) as *const i32)
    );
    crate::details::prepare_drawing_state();
}

/// TEMPORARY: how many dispatch lines are left before the log goes quiet, so a per-frame
/// hook cannot flood DebugView.
static DISPATCH_LOG_BUDGET: AtomicU32 = AtomicU32::new(12);

/// `this` comes from the register the replaced instruction read - edi in the update, esi
/// in the draw - so the page is taken from the object the game is actually dispatching on
/// rather than from the global, which may not be the same object.
#[no_mangle]
unsafe extern "C" fn church_update_hook(this: u32) -> i32 {
    let selected_page = *((this + 0x1d30) as *const i32);
    log_dispatch("update", this, selected_page);
    if selected_page == -1 {
        crate::details::invalidate(UIChurchWindowPtr { address: this });
    }
    selected_page
}

#[no_mangle]
unsafe extern "C" fn church_draw_hook(this: u32) -> i32 {
    let selected_page = *((this + 0x1d30) as *const i32);
    log_dispatch("draw", this, selected_page);
    if selected_page == -1 {
        crate::details::draw_page(UIChurchWindowPtr { address: this });
    }
    selected_page
}

/// TEMPORARY, remove once the page is confirmed.
unsafe fn log_dispatch(phase: &str, this: u32, page: i32) {
    if DISPATCH_LOG_BUDGET.fetch_sub(1, Ordering::Relaxed) == 0 {
        DISPATCH_LOG_BUDGET.store(0, Ordering::Relaxed);
        return;
    }
    info!(
        "church {phase}: this={this:#010x} global={:#010x} page={page}",
        *p3_api::ui::ui_church_window::STATIC_UI_CHURCH_WINDOW_PTR_ADDRESS
    );
}

extern "C" {
    static update_selected_page_detour: c_void;
    static draw_selected_page_detour: c_void;
}

// Both stubs stand in for `mov eax,[reg+0x1D30]`: call the Rust hook, which re-reads the
// page from the window global and returns it in eax, then continue into the compare and
// jump table exactly as the original would.
//
// ecx and edx are pushed around the call even though they are caller-saved: the game's own
// code did not expect a call here, so whatever it happens to be holding in them has to
// survive. This is the shape mod-tavern-details runs in play.
global_asm!("
.global {update_detour}
{update_detour}:
push ecx
push edx
push edi
call {update_hook}
add esp, 4
pop edx
pop ecx
jmp [{update_continuation}]

.global {draw_detour}
{draw_detour}:
push ecx
push edx
push esi
call {draw_hook}
add esp, 4
pop edx
pop ecx
jmp [{draw_continuation}]
",
    update_detour = sym update_selected_page_detour,
    update_hook = sym church_update_hook,
    update_continuation = sym UPDATE_SELECTED_PAGE_CONTINUATION,
    draw_detour = sym draw_selected_page_detour,
    draw_hook = sym church_draw_hook,
    draw_continuation = sym DRAW_SELECTED_PAGE_CONTINUATION,
);
