use crate::data::p3_ptr::P3Pointer;

/// The window manager: the static object the whole UI hangs off.
///
/// The game's main loop is `while (0x004B8A40(this = 0x006DA5F0) != -1)` (the loop
/// itself at `0x004B70C0`). Each frame that method pumps messages, updates the frame
/// clock (`0x004BD180`, 50 fps limiter included) and then calls the TOP window's
/// vtable `+0xF4` (update) and `+0x12C`.
///
/// The stack holds SCENES and full-screen menu screens only - the world map, the
/// town view, the main menu, the settings screen and its dialogs. **Building
/// windows (office, church, ...) and dialogs like the auto-trade goods dialog are
/// NOT on the stack**: they are children of the town scene, opened and closed
/// through their own vtable `+0x120`/`+0x118` without the manager ever seeing them
/// (verified live with entry detours on both manager methods, 27 Aug 2026 - the
/// depth never moved across thirteen office open/close cycles). Also verified: an
/// in-game load (settings -> load game) keeps all UI objects alive; only the
/// quit-to-menu teardown destroys them, without calling close - harmless, because
/// the settings screen closes building windows before it opens.
pub const WINDOW_MANAGER_ADDRESS: u32 = 0x006DA5F0;

/// The window stack is an MFC-style list inside the manager: `+0xC` points at the
/// TOP node, `+0x10` holds the depth, and each node is `{+0x4: link toward the
/// bottom, +0x8: the window object}`.
///
/// Windows enter and leave through two manager methods (both thiscall on
/// `0x006DA5F0`): **open/push** `0x004B90E0(window, arg)` activates the window
/// (vtable `+0xD4`, `+0x15C`) and inserts its node (`0x0064E6FC`), and
/// **remove** `0x004B9150(window)` finds the node from the top, unlinks it
/// (`0x0064E749`) and notifies the window (vtable `+0xD4`, `+0x160`). Hooking those
/// entries would give push/pop events; reading the stack answers the same questions
/// without any hook.
#[derive(Clone, Debug, Copy)]
pub struct WindowManagerPtr {
    pub address: u32,
}

impl Default for WindowManagerPtr {
    fn default() -> Self {
        Self::new()
    }
}

impl WindowManagerPtr {
    pub const fn new() -> Self {
        Self { address: WINDOW_MANAGER_ADDRESS }
    }

    /// How many windows the stack holds.
    pub fn get_depth(&self) -> u32 {
        unsafe { self.get(0x10) }
    }

    /// The window on top of the stack - the one whose update the frame loop runs,
    /// and the one that has the input.
    pub fn get_top_window(&self) -> Option<u32> {
        let node: u32 = unsafe { self.get(0x0c) };
        if !(0x0001_0000..0x7fff_0000).contains(&node) {
            return None;
        }
        let window = unsafe { *((node + 0x8) as *const u32) };
        if !(0x0001_0000..0x7fff_0000).contains(&window) {
            return None;
        }
        Some(window)
    }

    /// Is this window anywhere on the stack - top, or buried under a dialog?
    ///
    /// The walk the remove method does (`0x004B915D`..`0x004B917A`), bounded by the
    /// stack depth as a cycle guard.
    pub fn is_window_on_stack(&self, window: u32) -> bool {
        let mut node: u32 = unsafe { self.get(0x0c) };
        for _ in 0..self.get_depth().min(64) {
            if !(0x0001_0000..0x7fff_0000).contains(&node) {
                return false;
            }
            if unsafe { *((node + 0x8) as *const u32) } == window {
                return true;
            }
            node = unsafe { *((node + 0x4) as *const u32) };
        }
        false
    }
}

impl P3Pointer for WindowManagerPtr {
    fn get_address(&self) -> u32 {
        self.address
    }
}
