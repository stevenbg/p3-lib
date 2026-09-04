//! Client-side binding for the shared hotkey registry (`hotkey_registry.dll`, crate
//! `mod-hotkey-registry`).
//!
//! This is compiled into each consumer mod - correct on purpose: the binding holds
//! no shared state, only function pointers into the one registry, whose table lives
//! in hotkey_registry.dll itself. Bind once at `start()`, keep the [HotkeysApi] in a static,
//! and treat a failed bind as "keys inert": warn once and skip registration, never
//! fail the mod.
use std::{ffi::CStr, mem};

use windows::{
    core::{s, w},
    Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW},
};

/// The ABI this client speaks; [HotkeysApi::bind] refuses a registry that reports a
/// different version, because a silently mismatched signature across the DLL
/// boundary is stack corruption.
pub const API_VERSION: u32 = 1;

pub const MOD_SHIFT: u32 = 0x1;
pub const MOD_CTRL: u32 = 0x2;
pub const MOD_ALT: u32 = 0x4;

/// A key handler: receives the `(vk, mods)` that matched; nonzero = handled, which
/// swallows the keystroke (the game never sees it) and stops the LIFO walk.
pub type HotkeyHandler = unsafe extern "C" fn(vk: u32, mods: u32) -> u32;

type VersionFn = unsafe extern "C" fn() -> u32;
type RegisterFn = unsafe extern "C" fn(*const std::ffi::c_char, u32, u32, HotkeyHandler) -> u32;
type UnregisterFn = unsafe extern "C" fn(u32);

#[derive(Clone, Copy)]
pub struct HotkeysApi {
    register: RegisterFn,
    unregister: UnregisterFn,
}

impl HotkeysApi {
    /// Binds to `mods\hotkey_registry.dll`, loading it on demand: the modloader starts DLLs
    /// alphabetically, so a consumer that sorts earlier would find nothing with
    /// `GetModuleHandleW`. Registering works even before the registry's own
    /// `start()` has installed its keyboard hook. The library reference is never
    /// released - mod DLLs live for the whole process.
    ///
    /// # Safety
    /// Must run on the game main thread (mod `start()` and game callbacks are).
    pub unsafe fn bind() -> Result<Self, &'static str> {
        let module = LoadLibraryW(w!("mods\\hotkey_registry.dll")).map_err(|_| "hotkey_registry.dll not found in mods\\")?;
        let version: VersionFn =
            mem::transmute(GetProcAddress(module, s!("hotkeys_api_version")).ok_or("hotkeys_api_version export missing")?);
        if version() != API_VERSION {
            return Err("hotkey_registry.dll speaks a different ABI version");
        }
        let register: RegisterFn =
            mem::transmute(GetProcAddress(module, s!("hotkeys_register")).ok_or("hotkeys_register export missing")?);
        let unregister: UnregisterFn =
            mem::transmute(GetProcAddress(module, s!("hotkeys_unregister")).ok_or("hotkeys_unregister export missing")?);
        Ok(Self { register, unregister })
    }

    /// Registers `handler` for an exact `(vk, mods)` match at the TOP of the
    /// dispatch order and returns its handle (0 = registry table full). `owner` is
    /// a static mod name for the registry's logs.
    ///
    /// # Safety
    /// Game main thread only; the handler must stay valid forever (it does - mod
    /// DLLs never unload).
    pub unsafe fn register(&self, owner: &'static CStr, vk: u32, mods: u32, handler: HotkeyHandler) -> u32 {
        (self.register)(owner.as_ptr(), vk, mods, handler)
    }

    /// # Safety
    /// Game main thread only.
    pub unsafe fn unregister(&self, handle: u32) {
        (self.unregister)(handle)
    }
}
