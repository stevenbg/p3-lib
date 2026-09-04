//! The shared hotkey registry: one keyboard hook for all mods, so keys stop
//! colliding and window-scoped pages can claim keys that global features also use.
//!
//! The registry deliberately knows nothing about the game: it is a table, a
//! dispatcher and a log. Consumer mods register a key with a handler and get a
//! handle; they unregister the handle when their scope closes. Scoping is entirely
//! the consumer's job - register on window open / page enter, unregister on close /
//! leave (every needed lifecycle hook was play-verified; see
//! `.claude/notes/todo/hotkey-registry.md`). Dispatch is newest-first, so a key
//! registered by a window while it is open shadows a global registration of the
//! same key, and the global resurfaces when the window's registration is removed.
//! A handler returning nonzero swallows the keystroke (the game never sees it) and
//! stops the walk; a declining handler lets it continue. If nobody handles it, the
//! key passes to the game untouched.
//!
//! ## Binding, from a consumer mod
//!
//! C ABI only - no Rust types cross the DLL boundary. Bind dynamically so a missing
//! or stale hotkey_registry.dll degrades to a warning and inert keys instead of a load
//! failure, and bind with `LoadLibraryW(w"mods\\hotkey_registry.dll")` rather than
//! `GetModuleHandleW`: the modloader's start() order is alphabetical, so a consumer
//! that sorts earlier would otherwise bind before this DLL is loaded. LoadLibrary
//! loads it on demand (registrations into the table work before this DLL's own
//! `start()` has installed the hook) or just bumps the refcount; never free it.
//! Check `hotkeys_api_version()` against the version the consumer was compiled for
//! before anything else - a signature mismatch across the boundary is stack
//! corruption, a version check is one error line.
//!
//! ## The exports
//!
//! - `hotkeys_api_version() -> u32`
//! - `hotkeys_register(owner: *const c_char, vk: u32, mods: u32,
//!    handler: extern "C" fn(vk: u32, mods: u32) -> u32) -> u32` - returns a handle
//!    (0 = failure). Inserts at the top of the dispatch order. `owner` is a static
//!    NUL-terminated mod name, used only for the logs (safe: mod DLLs never unload).
//!    `mods` is an exact-match bitmask of [MOD_SHIFT] | [MOD_CTRL] | [MOD_ALT] -
//!    plain F1 does not fire while CTRL is held.
//! - `hotkeys_unregister(handle: u32)`
//!
//! Handlers receive the (vk, mods) that matched, so one handler can serve a mod's
//! whole key set. They run on the game's main thread, on the initial key-down only
//! (autorepeat and releases are filtered here), and may freely register and
//! unregister keys - dispatch snapshots the matches first and re-checks each handle
//! before calling.
//!
//! Everything runs on the game main thread - the modloader calls `start()` there,
//! the thread-scoped `WH_KEYBOARD` hook fires there, and consumers' lifecycle hooks
//! are game callbacks - so the table needs no locking; the single-thread invariant
//! is asserted in debug builds.
use std::{cell::UnsafeCell, ffi::c_char};

use log::{error, info, warn};
use windows::Win32::{
    Foundation::{LPARAM, LRESULT, WPARAM},
    System::Threading::GetCurrentThreadId,
    UI::{
        Input::KeyboardAndMouse::{GetKeyState, VIRTUAL_KEY, VK_CONTROL, VK_MENU, VK_SHIFT},
        WindowsAndMessaging::{CallNextHookEx, SetWindowsHookExW, HHOOK, WH_KEYBOARD},
    },
};

/// Bump on any breaking change to the exports or the handler signature.
pub const API_VERSION: u32 = 1;

pub const MOD_SHIFT: u32 = 0x1;
pub const MOD_CTRL: u32 = 0x2;
pub const MOD_ALT: u32 = 0x4;

/// A handler: nonzero = handled, swallow the keystroke and stop dispatch.
pub type HotkeyHandler = unsafe extern "C" fn(vk: u32, mods: u32) -> u32;

/// More than every current mod's keys combined; registration fails loudly at the cap
/// rather than allocating, so the table's memory never moves.
const CAPACITY: usize = 128;
/// A single keystroke rarely matches more than a couple of registrations.
const MAX_MATCHES: usize = 16;

#[derive(Clone, Copy)]
struct Entry {
    handle: u32,
    owner: *const c_char,
    vk: u32,
    mods: u32,
    handler: HotkeyHandler,
}

struct Table {
    /// Insertion-ordered; dispatch walks it backwards (newest first).
    entries: [Option<Entry>; CAPACITY],
    len: usize,
    next_handle: u32,
    main_thread: u32,
}

/// Single-threaded by construction (see the module docs); the wrapper exists because
/// a `static` must be `Sync`.
struct MainThreadOnly(UnsafeCell<Table>);
unsafe impl Sync for MainThreadOnly {}

static TABLE: MainThreadOnly = MainThreadOnly(UnsafeCell::new(Table {
    entries: [None; CAPACITY],
    len: 0,
    next_handle: 1,
    main_thread: 0,
}));

unsafe fn table() -> &'static mut Table {
    let table = &mut *TABLE.0.get();
    debug_assert!(
        table.main_thread == 0 || table.main_thread == GetCurrentThreadId(),
        "hotkeys registry touched off the game main thread"
    );
    table
}

unsafe fn owner_name(owner: *const c_char) -> &'static str {
    if owner.is_null() {
        return "?";
    }
    std::ffi::CStr::from_ptr(owner).to_str().unwrap_or("?")
}

fn describe(vk: u32, mods: u32) -> String {
    let mut name = String::new();
    if mods & MOD_CTRL != 0 {
        name.push_str("CTRL+");
    }
    if mods & MOD_ALT != 0 {
        name.push_str("ALT+");
    }
    if mods & MOD_SHIFT != 0 {
        name.push_str("SHIFT+");
    }
    name.push_str(&format!("vk{vk:#04x}"));
    name
}

#[no_mangle]
pub unsafe extern "C" fn start() -> u32 {
    let _ = log::set_logger(&win_dbg_logger::DEBUGGER_LOGGER);
    log::set_max_level(log::LevelFilter::Info);

    let table = table();
    table.main_thread = GetCurrentThreadId();

    if SetWindowsHookExW(WH_KEYBOARD, Some(keyboard_hook), None, GetCurrentThreadId()).is_err() {
        error!("failed to install the keyboard hook");
        return 1;
    }

    // Consumers that sort before this DLL may have registered already (they
    // LoadLibrary it on demand); dump what the table holds at hook activation.
    if table.len > 0 {
        for entry in table.entries[..table.len].iter().flatten() {
            info!("key map at start: {} -> {}", describe(entry.vk, entry.mods), owner_name(entry.owner));
        }
    }
    info!("hotkey registry live (api version {API_VERSION}, {} keys registered)", table.len);
    0
}

#[no_mangle]
pub extern "C" fn hotkeys_api_version() -> u32 {
    API_VERSION
}

/// Registers a handler for an exact (vk, mods) match and returns its handle
/// (0 = table full). The new registration goes to the TOP of the dispatch order:
/// it shadows every earlier registration of the same key until unregistered.
/// Shadowing is a feature and is logged as information, not as a conflict.
#[no_mangle]
pub unsafe extern "C" fn hotkeys_register(owner: *const c_char, vk: u32, mods: u32, handler: HotkeyHandler) -> u32 {
    let table = table();
    if table.len == CAPACITY {
        error!("hotkeys table full ({CAPACITY}): {} cannot register {}", owner_name(owner), describe(vk, mods));
        return 0;
    }

    for entry in table.entries[..table.len].iter().flatten() {
        if entry.vk == vk && entry.mods == mods {
            info!(
                "{}: {} now shadows {}",
                describe(vk, mods),
                owner_name(owner),
                owner_name(entry.owner),
            );
        }
    }

    let handle = table.next_handle;
    table.next_handle += 1;
    let len = table.len;
    table.entries[len] = Some(Entry { handle, owner, vk, mods, handler });
    table.len += 1;
    info!("registered {} -> {} (handle {handle})", describe(vk, mods), owner_name(owner));
    handle
}

#[no_mangle]
pub unsafe extern "C" fn hotkeys_unregister(handle: u32) {
    let table = table();
    let Some(index) = table.entries[..table.len]
        .iter()
        .position(|entry| entry.is_some_and(|e| e.handle == handle))
    else {
        warn!("hotkeys_unregister: handle {handle} not registered");
        return;
    };
    let entry = table.entries[index].unwrap();
    info!("unregistered {} -> {} (handle {handle})", describe(entry.vk, entry.mods), owner_name(entry.owner));
    for i in index..table.len - 1 {
        table.entries[i] = table.entries[i + 1];
    }
    table.len -= 1;
    table.entries[table.len] = None;
}

fn key_down(key: VIRTUAL_KEY) -> bool {
    (unsafe { GetKeyState(key.0 as i32) } as u16) & 0x8000 != 0
}

unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let flags = lparam.0 as u32;
        // Bit 31: transition state (0 = key pressed); bit 30: previous state (0 = was
        // up). Together: fire once on the initial key-down, never on autorepeat or
        // release - handlers see one call per physical press.
        if flags & 0xC000_0000 == 0 {
            let vk = wparam.0 as u32;
            let mut mods = 0;
            if key_down(VK_SHIFT) {
                mods |= MOD_SHIFT;
            }
            if key_down(VK_CONTROL) {
                mods |= MOD_CTRL;
            }
            if key_down(VK_MENU) {
                mods |= MOD_ALT;
            }

            // Snapshot the matches newest-first before calling anything: a handler
            // may register or unregister keys (opening a window from a hotkey arms
            // that window's keys), which would invalidate a live iteration. Each
            // handle is re-checked before its call so a handler that unregisters a
            // later match also disarms it for this very keystroke.
            let table = table();
            let mut matches = [None; MAX_MATCHES];
            let mut count = 0;
            for entry in table.entries[..table.len].iter().rev().flatten() {
                if entry.vk == vk && entry.mods == mods {
                    if count == MAX_MATCHES {
                        warn!("more than {MAX_MATCHES} registrations for {}", describe(vk, mods));
                        break;
                    }
                    matches[count] = Some((entry.handle, entry.handler));
                    count += 1;
                }
            }
            for (handle, handler) in matches[..count].iter().flatten() {
                let current = self::table();
                let still_registered = current.entries[..current.len]
                    .iter()
                    .any(|entry| entry.is_some_and(|e| e.handle == *handle));
                if !still_registered {
                    continue;
                }
                if handler(vk, mods) != 0 {
                    // Handled: eat the keystroke - the game and later hooks in the
                    // chain never see it.
                    return LRESULT(1);
                }
            }
        }
    }
    CallNextHookEx(HHOOK(0), code, wparam, lparam)
}
