//! The numeric input box (the amount and price columns of the trading office, the goods
//! dialog's amounts): a text widget subclass that only accepts digits and keeps the parsed
//! value beside the text.
//!
//! **Keyboard focus** is the scene container's business: its click dispatcher keeps the index
//! of the last clicked child in `+0xC4` and calls the old child's `+0xC0` (focus lost) and the
//! new child's `+0xBC` (focus gained, `0x004CA640`: cursor at the end, byte `+0x42` = 1). The
//! main window's `WM_KEYDOWN` handler (`0x004C0BE0`) hands every key to the window stack
//! (`0x004B92E0`), which forwards it to the top scene's key slot `+0x1C`, and from there it
//! reaches the focused child's own `+0x1C`. This class's key slot `0x0045C300` returns at once
//! unless `+0x42` is set, translates the virtual key with `ToAscii` (`0x004C0F50`), keeps only
//! `'0'..'9'`, backspace and delete, edits the text and stores `atoi(text)` in `+0x188`.
//!
//! The owning window turns the value into game state itself: the trading office's page update
//! (`0x005DCDB0`) enqueues operation `0x5B` every frame for a row whose amount or price box is
//! focused, with the box's `+0x188` and the direction of the row's arrow buttons. So a mod
//! that sets a focused box's value with [`NumberWidget::set_value`] has changed the order the
//! same way typing would.

/// The class vtable; slot `+0x1C` is the key handler `thiscall(vk, repeat, flags)`, `ret 0xC`.
pub const VTABLE: u32 = 0x0066_DBB8;
pub const SLOT_KEY: usize = 0x1C;
pub const OBJECT_SIZE: u32 = 0x190;
/// `thiscall(value)`, `ret 4`: clamp to `+0x184..=+0x180`, store at `+0x188`, flag `+0x18C`
/// dirty when changed, and rewrite the text.
const SET_VALUE: u32 = 0x0045_C930;
const FIELD_FOCUSED: u32 = 0x42;
const FIELD_MAX: u32 = 0x180;
const FIELD_MIN: u32 = 0x184;
const FIELD_VALUE: u32 = 0x188;

/// A handle to one of the game's number boxes.
#[derive(Clone, Copy, Debug)]
pub struct NumberWidget {
    pub address: u32,
}

impl NumberWidget {
    pub fn new(address: u32) -> Self {
        Self { address }
    }

    /// The displayed value, in the box's own units (the trading office shows in-game units).
    pub unsafe fn value(&self) -> i32 {
        *((self.address + FIELD_VALUE) as *const i32)
    }

    /// Inclusive bounds the setter clamps to.
    pub unsafe fn bounds(&self) -> (i32, i32) {
        (*((self.address + FIELD_MIN) as *const i32), *((self.address + FIELD_MAX) as *const i32))
    }

    pub unsafe fn set_value(&self, value: i32) {
        let set: extern "thiscall" fn(u32, i32) = std::mem::transmute(SET_VALUE);
        set(self.address, value);
    }

    /// Whether the box has the keyboard (was clicked last in its scene).
    pub unsafe fn is_focused(&self) -> bool {
        *((self.address + FIELD_FOCUSED) as *const u8) != 0
    }
}
