//! THROWAWAY probe: log every window-manager transition and the office window's own
//! lifecycle, to settle what quitting to the main menu and loading a save do to open
//! windows.
//!
//! Why: the hotkey-registry design (`.claude/notes/todo/hotkey-registry.md`) makes
//! scoping the consumer mod's job - register keys on window open, unregister on close.
//! Its one failure mode is a close that never fires. The ordinary close paths are easy
//! to try; the two suspects are **quit to main menu** and **loading a save with a
//! window open**, whose teardown is a giant SEH destructor (`0x00427xxx`) that was
//! never traced. This probe watches instead of disassembling.
//!
//! Five event sources, all passive - no hotkeys, just play:
//!
//! - `push` / `remove`: entry detours on the window manager's two transition methods
//!   (`0x004B90E0` / `0x004B9150`, thiscall on `0x006DA5F0`, the window as the first
//!   stack argument). Every stack entry and exit, whoever triggers it.
//! - `open` / `close` / `dtor`: vtable hooks on the trading-office window class
//!   (slots `+0x120` / `+0x118` / `+0x0`) - the class auto-supply scopes its keys by,
//!   so its close is the one the registry design would rely on.
//!
//! Every line carries the window pointer (labelled when it matches a known global),
//! the stack depth, and a unix timestamp, appended to `_window_lifecycle.log` in the
//! game folder. What to look for after playing the scenarios (ESC-close, close via
//! another building, load-with-window-open, quit-to-menu-with-window-open):
//!
//! - does every `open` have a matching `close`, on every path?
//! - does the teardown `remove` windows from the stack, `close` them, or only `dtor`
//!   them?
//! - does anything leave the stack without passing `0x004B9150`? (depth jumping down
//!   by more than the removes seen would show that.)
use std::{ffi::c_void, io::Write, mem, sync::atomic::{AtomicI32, AtomicPtr, Ordering}};

use hooklet::windows::x86::{deploy_rel32_raw, hook_function_pointer, FunctionPointerHook, X86Rel32Type};
use log::{debug, error, info};
use p3_api::ui::{
    ui_trading_office_window::{UITradingOfficeWindowPtr, STATIC_UI_TRADING_OFFICE_WINDOW_PTR_ADDRESS},
    window_manager::WindowManagerPtr,
};

/// The window manager's push (`0x004B90E0(window, arg)`): stolen prologue
/// `sub esp,0x1C; push esi; mov esi,[esp+0x24]` (8 bytes - the detour overwrites the
/// first 5 and the stub re-executes all three instructions).
const PUSH_PATCH_ADDRESS: u32 = 0x004B90E0;
static PUSH_CONTINUATION: u32 = 0x004B90E8;
/// The remove (`0x004B9150(window)`): stolen prologue `sub esp,0xC; push ebx; push ebp`
/// (5 bytes exactly) - the recipe proven in mod-battle-speed.
const REMOVE_PATCH_ADDRESS: u32 = 0x004B9150;
static REMOVE_CONTINUATION: u32 = 0x004B9155;

/// The office window's vtable (module-relative, as `hook_function_pointer` takes it):
/// slot 0 is the scalar deleting destructor (the teardown calls it with flags 3),
/// `+0x118` close, `+0x120` open.
const OFFICE_DTOR_POINTER_OFFSET: u32 = UITradingOfficeWindowPtr::VTABLE_OFFSET;
const OFFICE_CLOSE_POINTER_OFFSET: u32 = UITradingOfficeWindowPtr::VTABLE_OFFSET + 0x118;
const OFFICE_OPEN_POINTER_OFFSET: u32 = UITradingOfficeWindowPtr::VTABLE_OFFSET + 0x120;

/// Known window globals, to label pointers in the log. `0x006E556C` is the church
/// window (established for mod-fix-church-anim-crash); the office one is p3-api's.
const CHURCH_WINDOW_PTR: *const u32 = 0x006E556C as _;

/// The auto-trade goods dialog: NOT a manager window (round 1 finding) - a child
/// widget of the scrollmap ship panel with its own vtable at `0x0066A7F0`. Its close
/// (`+0x118` = `0x004066F0`) resets the stop index `+0xA4` to -1, which is the field
/// `mod-auto-supply`'s scoping predicate reads - so these hooks answer whether every
/// path that visibly hides the dialog actually runs close, or whether a hide can
/// leave the predicate reading "open". `+0xCC` is the shared base-class show(flag),
/// writing the visibility byte `+0x48`; hooking the dialog's SLOT scopes it to the
/// dialog alone.
const GOODS_DIALOG_PTR: *const u32 = 0x006CBA74 as _;
const DIALOG_VTABLE_OFFSET: u32 = 0x0026A7F0;
const DIALOG_DTOR_POINTER_OFFSET: u32 = DIALOG_VTABLE_OFFSET;
const DIALOG_SHOW_POINTER_OFFSET: u32 = DIALOG_VTABLE_OFFSET + 0xCC;
const DIALOG_CLOSE_POINTER_OFFSET: u32 = DIALOG_VTABLE_OFFSET + 0x118;
const DIALOG_STOP_OFFSET: u32 = 0xA4;
const DIALOG_VISIBLE_OFFSET: u32 = 0x48;

/// The tavern window, for the page-transition question: can the selected page
/// (`+0x1BF4`) change anywhere outside open / close / update / set_page? The page
/// field has 14 direct writers plus the `set_page` method (`0x005CED00`, eleven
/// callers), all statically reachable only from those methods - this instrumentation
/// verifies that claim in play. The shadow holds the last page any hook observed;
/// a mismatch at update ENTRY means a transition escaped every hook.
const TAVERN_VTABLE_OFFSET: u32 = 0x279B78;
const TAVERN_OPEN_POINTER_OFFSET: u32 = TAVERN_VTABLE_OFFSET + 0x120;
const TAVERN_CLOSE_POINTER_OFFSET: u32 = TAVERN_VTABLE_OFFSET + 0x118;
const TAVERN_UPDATE_POINTER_OFFSET: u32 = TAVERN_VTABLE_OFFSET + 0xF4;
const TAVERN_PAGE_OFFSET: u32 = 0x1BF4;
/// `set_page(page)`: stolen prologue `push -1; push 0x00666C8C` (7 bytes).
const SET_PAGE_PATCH_ADDRESS: u32 = 0x005CED00;
static SET_PAGE_CONTINUATION: u32 = 0x005CED07;

const LOG_FILE: &str = "_window_lifecycle.log";

static OPEN_HOOK: AtomicPtr<FunctionPointerHook> = AtomicPtr::new(std::ptr::null_mut());
static CLOSE_HOOK: AtomicPtr<FunctionPointerHook> = AtomicPtr::new(std::ptr::null_mut());
static DTOR_HOOK: AtomicPtr<FunctionPointerHook> = AtomicPtr::new(std::ptr::null_mut());
static DIALOG_DTOR_HOOK: AtomicPtr<FunctionPointerHook> = AtomicPtr::new(std::ptr::null_mut());
static DIALOG_SHOW_HOOK: AtomicPtr<FunctionPointerHook> = AtomicPtr::new(std::ptr::null_mut());
static DIALOG_CLOSE_HOOK: AtomicPtr<FunctionPointerHook> = AtomicPtr::new(std::ptr::null_mut());
static TAVERN_OPEN_HOOK: AtomicPtr<FunctionPointerHook> = AtomicPtr::new(std::ptr::null_mut());
static TAVERN_CLOSE_HOOK: AtomicPtr<FunctionPointerHook> = AtomicPtr::new(std::ptr::null_mut());
static TAVERN_UPDATE_HOOK: AtomicPtr<FunctionPointerHook> = AtomicPtr::new(std::ptr::null_mut());
/// The last page value any hook observed, i32; i64::MIN-ish sentinel start.
static TAVERN_PAGE_SHADOW: AtomicI32 = AtomicI32::new(-2);

#[no_mangle]
pub unsafe extern "C" fn start() -> u32 {
    let _ = log::set_logger(&win_dbg_logger::DEBUGGER_LOGGER);
    log::set_max_level(log::LevelFilter::Debug);

    if deploy_rel32_raw(PUSH_PATCH_ADDRESS as _, (&push_detour) as *const _ as _, X86Rel32Type::Jump).is_err() {
        error!("failed to detour the window manager push at {PUSH_PATCH_ADDRESS:#010x}");
        return 1;
    }
    if deploy_rel32_raw(REMOVE_PATCH_ADDRESS as _, (&remove_detour) as *const _ as _, X86Rel32Type::Jump).is_err() {
        error!("failed to detour the window manager remove at {REMOVE_PATCH_ADDRESS:#010x}");
        return 2;
    }
    match hook_function_pointer(OFFICE_OPEN_POINTER_OFFSET, office_open_hook as usize as u32) {
        Ok(hook) => OPEN_HOOK.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
        Err(_) => {
            error!("failed to hook the office window open slot");
            return 3;
        }
    }
    match hook_function_pointer(OFFICE_CLOSE_POINTER_OFFSET, office_close_hook as usize as u32) {
        Ok(hook) => CLOSE_HOOK.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
        Err(_) => {
            error!("failed to hook the office window close slot");
            return 4;
        }
    }
    match hook_function_pointer(OFFICE_DTOR_POINTER_OFFSET, office_dtor_hook as usize as u32) {
        Ok(hook) => DTOR_HOOK.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
        Err(_) => {
            error!("failed to hook the office window dtor slot");
            return 5;
        }
    }

    match hook_function_pointer(DIALOG_DTOR_POINTER_OFFSET, dialog_dtor_hook as usize as u32) {
        Ok(hook) => DIALOG_DTOR_HOOK.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
        Err(_) => {
            error!("failed to hook the goods dialog dtor slot");
            return 6;
        }
    }
    match hook_function_pointer(DIALOG_SHOW_POINTER_OFFSET, dialog_show_hook as usize as u32) {
        Ok(hook) => DIALOG_SHOW_HOOK.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
        Err(_) => {
            error!("failed to hook the goods dialog show slot");
            return 7;
        }
    }
    match hook_function_pointer(DIALOG_CLOSE_POINTER_OFFSET, dialog_close_hook as usize as u32) {
        Ok(hook) => DIALOG_CLOSE_HOOK.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
        Err(_) => {
            error!("failed to hook the goods dialog close slot");
            return 8;
        }
    }

    match hook_function_pointer(TAVERN_OPEN_POINTER_OFFSET, tavern_open_hook as usize as u32) {
        Ok(hook) => TAVERN_OPEN_HOOK.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
        Err(_) => {
            error!("failed to hook the tavern open slot");
            return 9;
        }
    }
    match hook_function_pointer(TAVERN_CLOSE_POINTER_OFFSET, tavern_close_hook as usize as u32) {
        Ok(hook) => TAVERN_CLOSE_HOOK.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
        Err(_) => {
            error!("failed to hook the tavern close slot");
            return 10;
        }
    }
    match hook_function_pointer(TAVERN_UPDATE_POINTER_OFFSET, tavern_update_hook as usize as u32) {
        Ok(hook) => TAVERN_UPDATE_HOOK.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
        Err(_) => {
            error!("failed to hook the tavern update slot");
            return 11;
        }
    }
    if deploy_rel32_raw(SET_PAGE_PATCH_ADDRESS as _, (&set_page_detour) as *const _ as _, X86Rel32Type::Jump).is_err() {
        error!("failed to detour tavern set_page at {SET_PAGE_PATCH_ADDRESS:#010x}");
        return 12;
    }

    info!("window lifecycle probe installed; events go to {LOG_FILE}");
    append_log_line("=== probe loaded (new game session process) ===");
    0
}

/// The dialog's stop index and visibility byte, the two fields whose agreement the
/// scoping predicate depends on.
unsafe fn dialog_state(dialog: u32) -> String {
    let stop = *((dialog + DIALOG_STOP_OFFSET) as *const i32);
    let visible = *((dialog + DIALOG_VISIBLE_OFFSET) as *const u8);
    format!("stop={stop} visible={visible}")
}

#[no_mangle]
unsafe extern "thiscall" fn dialog_close_hook(dialog: u32) {
    let line = format!("dlg-close  dialog={dialog:#010x} before: {}", dialog_state(dialog));
    debug!("{line}");
    append_log_line(&line);
    let orig: extern "thiscall" fn(u32) = mem::transmute((*DIALOG_CLOSE_HOOK.load(Ordering::SeqCst)).old_absolute);
    orig(dialog);
}

/// The shared base-class show(flag); hooking the dialog's vtable slot scopes it to
/// the dialog. Logs hide/show WITH the stop index, so a hide that leaves the stop
/// set (a predicate false positive) is visible directly.
#[no_mangle]
unsafe extern "thiscall" fn dialog_show_hook(dialog: u32, flag: u32) {
    let line = format!("dlg-show   dialog={dialog:#010x} flag={flag} {}", dialog_state(dialog));
    debug!("{line}");
    append_log_line(&line);
    let orig: extern "thiscall" fn(u32, u32) = mem::transmute((*DIALOG_SHOW_HOOK.load(Ordering::SeqCst)).old_absolute);
    orig(dialog, flag);
}

#[no_mangle]
unsafe extern "thiscall" fn dialog_dtor_hook(dialog: u32, flags: u32) -> u32 {
    let line = format!("dlg-dtor   dialog={dialog:#010x} flags={flags} {}", dialog_state(dialog));
    debug!("{line}");
    append_log_line(&line);
    let orig: extern "thiscall" fn(u32, u32) -> u32 = mem::transmute((*DIALOG_DTOR_HOOK.load(Ordering::SeqCst)).old_absolute);
    orig(dialog, flags)
}

/// Labels a window pointer when it matches a known global.
unsafe fn label(window: u32) -> &'static str {
    if window == 0 {
        return " (NULL)";
    }
    if window == *STATIC_UI_TRADING_OFFICE_WINDOW_PTR_ADDRESS {
        return " (office)";
    }
    if window == *CHURCH_WINDOW_PTR {
        return " (church)";
    }
    ""
}

unsafe fn log_event(event: &str, window: u32) {
    let depth = WindowManagerPtr::new().get_depth();
    let line = format!("{event} window={window:#010x}{} depth={depth}", label(window));
    debug!("{line}");
    append_log_line(&line);
}

/// Called from the two asm stubs with the window argument. cdecl; ecx/edx preserved
/// by the stubs themselves.
#[no_mangle]
unsafe extern "C" fn manager_transition(site: u32, window: u32) {
    log_event(if site == 0 { "push  " } else { "remove" }, window);
}

#[no_mangle]
unsafe extern "thiscall" fn office_open_hook(window: u32) {
    log_event("open  ", window);
    let orig: extern "thiscall" fn(u32) = mem::transmute((*OPEN_HOOK.load(Ordering::SeqCst)).old_absolute);
    orig(window);
}

#[no_mangle]
unsafe extern "thiscall" fn office_close_hook(window: u32) {
    log_event("close ", window);
    let orig: extern "thiscall" fn(u32) = mem::transmute((*CLOSE_HOOK.load(Ordering::SeqCst)).old_absolute);
    orig(window);
}

/// The scalar deleting destructor: `flags & 1` frees the memory. Log BEFORE calling
/// through - the object is gone afterwards.
#[no_mangle]
unsafe extern "thiscall" fn office_dtor_hook(window: u32, flags: u32) -> u32 {
    let depth = WindowManagerPtr::new().get_depth();
    let line = format!("dtor   window={window:#010x}{} depth={depth} flags={flags}", label(window));
    debug!("{line}");
    append_log_line(&line);
    let orig: extern "thiscall" fn(u32, u32) -> u32 = mem::transmute((*DTOR_HOOK.load(Ordering::SeqCst)).old_absolute);
    orig(window, flags)
}

unsafe fn tavern_page(window: u32) -> i32 {
    *((window + TAVERN_PAGE_OFFSET) as *const i32)
}

fn log_line(line: &str) {
    debug!("{line}");
    append_log_line(line);
}

#[no_mangle]
unsafe extern "thiscall" fn tavern_open_hook(window: u32) {
    let orig: extern "thiscall" fn(u32) = mem::transmute((*TAVERN_OPEN_HOOK.load(Ordering::SeqCst)).old_absolute);
    orig(window);
    let page = tavern_page(window);
    TAVERN_PAGE_SHADOW.store(page, Ordering::Relaxed);
    log_line(&format!("tav-open   page={page}"));
}

#[no_mangle]
unsafe extern "thiscall" fn tavern_close_hook(window: u32) {
    let before = tavern_page(window);
    let orig: extern "thiscall" fn(u32) = mem::transmute((*TAVERN_CLOSE_HOOK.load(Ordering::SeqCst)).old_absolute);
    orig(window);
    let page = tavern_page(window);
    TAVERN_PAGE_SHADOW.store(page, Ordering::Relaxed);
    log_line(&format!("tav-close  page {before} -> {page}"));
}

/// The per-frame update: the miss-detector at entry (a page differing from the
/// shadow means a transition escaped every hook since the last frame), the
/// transition log across the original call.
#[no_mangle]
unsafe extern "thiscall" fn tavern_update_hook(window: u32) -> u32 {
    let entry = tavern_page(window);
    let shadow = TAVERN_PAGE_SHADOW.load(Ordering::Relaxed);
    if entry != shadow && shadow != -2 {
        log_line(&format!("tav-MISSED page changed outside hooks: shadow {shadow}, found {entry}"));
    }
    let orig: extern "thiscall" fn(u32) -> u32 = mem::transmute((*TAVERN_UPDATE_HOOK.load(Ordering::SeqCst)).old_absolute);
    let result = orig(window);
    let after = tavern_page(window);
    if after != entry {
        log_line(&format!("tav-page   {entry} -> {after} (update)"));
    }
    TAVERN_PAGE_SHADOW.store(after, Ordering::Relaxed);
    result
}

/// Called from the set_page asm stub. cdecl; ecx/edx preserved by the stub.
#[no_mangle]
unsafe extern "C" fn set_page_logger(window: u32, page: u32) {
    let current = tavern_page(window);
    TAVERN_PAGE_SHADOW.store(page as i32, Ordering::Relaxed);
    log_line(&format!("tav-set    set_page({}) from {current}", page as i32));
}

fn append_log_line(line: &str) {
    let unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(LOG_FILE) {
        let _ = writeln!(file, "[unix {unix}] {line}");
    }
}

extern "C" {
    static push_detour: c_void;
    static remove_detour: c_void;
    static set_page_detour: c_void;
}

// set_page(this=ecx, page at [esp+4]): log at entry, preserve ecx/edx, re-execute
// the stolen SEH prologue, continue.
global_asm!("
.global {set_page_detour}
{set_page_detour}:
push ecx
push edx
mov eax, dword ptr [esp + 12]
push eax
push ecx
call {logger}
add esp, 8
pop edx
pop ecx
push -1
push 0x00666C8C
jmp [{set_page_continuation}]
",
set_page_detour = sym set_page_detour,
logger = sym set_page_logger,
set_page_continuation = sym SET_PAGE_CONTINUATION);

// Both stubs run at the very top of their manager method: ecx is the manager, the
// window is the first stack argument. ecx and edx must survive the logger call (the
// prologues replayed below use neither, but the bodies read ecx); eax is dead.
// The logger is cdecl: arguments pushed in reverse.
global_asm!("
.global {push_detour}
{push_detour}:
push ecx
push edx
mov eax, dword ptr [esp + 12]
push eax
push 0
call {logger}
add esp, 8
pop edx
pop ecx
# the three instructions the jmp overwrote, then back into the original
sub esp, 0x1C
push esi
mov esi, dword ptr [esp + 0x24]
jmp [{push_continuation}]
",
push_detour = sym push_detour,
logger = sym manager_transition,
push_continuation = sym PUSH_CONTINUATION);

global_asm!("
.global {remove_detour}
{remove_detour}:
push ecx
push edx
mov eax, dword ptr [esp + 12]
push eax
push 1
call {logger}
add esp, 8
pop edx
pop ecx
# the three instructions the jmp overwrote, then back into the original
sub esp, 0xC
push ebx
push ebp
jmp [{remove_continuation}]
",
remove_detour = sym remove_detour,
logger = sym manager_transition,
remove_continuation = sym REMOVE_CONTINUATION);

use std::arch::global_asm;
