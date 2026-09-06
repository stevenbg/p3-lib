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

//!
//! **Building one** is what the trading office's ini loader (`0x005D9710`, loop at
//! `0x005D9838`) does for each of its forty boxes: construct (`0x0045C110`), load
//! `[TextBox<id>]` from `./scripts/BuildingParchment.ini` through the text widget's loader
//! (vtable `+0x8`, `0x004C9A70`, the section name formatted from the id), give it a fresh
//! `0x20`-byte helper object of class `0x004B03E0` through vtable `+0x10` (`0x004D2FB0`),
//! set its mode to 0 (`0x004D2E20`) and its text colour to opaque black (`0x004CABC0`).
//! The office's amount and price boxes are all `[TextBox1]`; the open method then caps
//! them at 9999 (`0x005D91CA`). [NumberWidget::build] replays exactly that.

use crate::data::fill_p3_string;

use super::widget;

/// The class vtable; slot `+0x1C` is the key handler `thiscall(vk, repeat, flags)`, `ret 0xC`.
pub const VTABLE: u32 = 0x0066_DBB8;
pub const SLOT_KEY: usize = 0x1C;
pub const OBJECT_SIZE: u32 = 0x190;
/// `thiscall(value)`, `ret 4`: clamp to `+0x184..=+0x180`, store at `+0x188`, flag `+0x18C`
/// dirty when changed, and rewrite the text.
const SET_VALUE: u32 = 0x0045_C930;
/// `thiscall()`: the text widget's constructor, then the vtable, bounds 0/0, value -1.
const CTOR: u32 = 0x0045_C110;
/// Vtable `+0x8`, `thiscall(CString by value, id)`, `ret 8`: load `[TextBox<id>]` from the
/// ini. Consumes the string.
const LOAD: u32 = 0x004C_9A70;
/// Vtable `+0x10`, `thiscall(helper)`, `ret 4`: hand the box its helper object
/// (`0x004B1890` on the base, then the `+0x118` refresh).
const SET_HELPER: u32 = 0x004D_2FB0;
/// `thiscall(mode)`, `ret 4`: `+0xD8`, then the `+0x118` refresh. The office passes 0.
const SET_MODE: u32 = 0x004D_2E20;
/// `thiscall(argb)`, `ret 4`: text colour at `+0x158`, its alpha byte at `+0x15C`.
const SET_TEXT_COLOR: u32 = 0x004C_ABC0;
/// The helper's class: `0x20` bytes, `thiscall()` constructor, one per box.
const HELPER_CTOR: u32 = 0x004B_03E0;
const HELPER_SIZE: usize = 0x20;
const FIELD_FOCUSED: u32 = 0x42;
const FIELD_MAX: u32 = 0x180;
const FIELD_MIN: u32 = 0x184;
const FIELD_VALUE: u32 = 0x188;
/// The shared empty-string block; `+0xC` is the `CString` value for "".
const NIL_STRING_HEADER: *const u32 = 0x006C_7CD0 as _;

/// The look of the trading office's amount and price boxes.
pub const OFFICE_BOX_INI: &[u8] = b"./scripts/BuildingParchment.ini\0";
pub const OFFICE_BOX_ID: u32 = 1;
/// The bound the office puts on its boxes.
pub const OFFICE_BOX_MAX: i32 = 9999;
const OPAQUE_BLACK: u32 = 0xFF00_0000;

/// Our field, past the game's object.
const FIELD_ATTACHED: u32 = OBJECT_SIZE;
const BUFFER_SIZE: usize = OBJECT_SIZE as usize + 8;

#[repr(align(8))]
struct Buffer {
    _bytes: [u8; BUFFER_SIZE],
}

#[repr(align(8))]
struct HelperBuffer {
    _bytes: [u8; HELPER_SIZE],
}

/// A handle to a number box - one of the game's, or one built with [NumberWidget::build].
#[derive(Clone, Copy, Debug)]
pub struct NumberWidget {
    pub address: u32,
}

impl NumberWidget {
    pub fn new(address: u32) -> Self {
        Self { address }
    }

    /// Build a box of our own the way the trading office builds its: `[TextBox<id>]` of
    /// `ini` (a NUL-terminated `./scripts/...` path), its helper, mode 0, black text, bounds
    /// `0..=max`, value 0. Hidden and unplaced until [NumberWidget::set_position] and
    /// [NumberWidget::show]; register it with [NumberWidget::attach_to_root] after the window
    /// it rides on, and it draws itself and takes the keyboard when clicked like the game's
    /// own. The object lives for the rest of the process.
    pub unsafe fn build(ini: &[u8], id: u32, max: i32) -> Self {
        let address = Box::leak(Box::new(Buffer { _bytes: [0; BUFFER_SIZE] })) as *mut Buffer as u32;
        let ctor: extern "thiscall" fn(u32) = std::mem::transmute(CTOR);
        ctor(address);
        // The loader takes the string by value and releases it: hand it a fresh one.
        let mut ini_string = *NIL_STRING_HEADER + 0xC;
        fill_p3_string((&mut ini_string) as *mut u32 as _, ini);
        let load: extern "thiscall" fn(u32, u32, u32) = std::mem::transmute(LOAD);
        load(address, ini_string, id);

        let helper = Box::leak(Box::new(HelperBuffer { _bytes: [0; HELPER_SIZE] })) as *mut HelperBuffer as u32;
        let helper_ctor: extern "thiscall" fn(u32) = std::mem::transmute(HELPER_CTOR);
        helper_ctor(helper);
        let set_helper: extern "thiscall" fn(u32, u32) = std::mem::transmute(SET_HELPER);
        set_helper(address, helper);
        let set_mode: extern "thiscall" fn(u32, u32) = std::mem::transmute(SET_MODE);
        set_mode(address, 0);
        let set_color: extern "thiscall" fn(u32, u32) = std::mem::transmute(SET_TEXT_COLOR);
        set_color(address, OPAQUE_BLACK);

        let widget = Self { address };
        *((address + FIELD_MIN) as *mut i32) = 0;
        *((address + FIELD_MAX) as *mut i32) = max;
        *((address + FIELD_ATTACHED) as *mut u32) = 0;
        widget.set_value(0);
        widget
    }

    pub unsafe fn set_position(&self, x: i32, y: i32) {
        widget::set_position(self.address, x, y);
    }

    pub unsafe fn position(&self) -> (i32, i32) {
        widget::position(self.address)
    }

    /// Width and height, from the ini section.
    pub unsafe fn size(&self) -> (i32, i32) {
        widget::size(self.address)
    }

    pub unsafe fn show(&self, visible: bool) {
        widget::show(self.address, visible);
    }

    pub unsafe fn is_visible(&self) -> bool {
        widget::is_visible(self.address)
    }

    /// Only meaningful for a box from [NumberWidget::build].
    pub fn is_attached(&self) -> bool {
        unsafe { *((self.address + FIELD_ATTACHED) as *const u32) != 0 }
    }

    /// Register with the top scene, after the window the box rides on.
    pub unsafe fn attach_to_root(&self) {
        if widget::add_to_root(self.address) {
            *((self.address + FIELD_ATTACHED) as *mut u32) = 1;
        }
    }

    pub unsafe fn detach(&self) {
        widget::remove_from_root(self.address);
        *((self.address + FIELD_ATTACHED) as *mut u32) = 0;
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
