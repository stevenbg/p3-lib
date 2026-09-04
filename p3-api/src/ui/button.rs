//! `CViperButton` - the game's button: four looks (disabled, neutral, pressed, hover - each a
//! `C2DAnimation`), a text, and one of three behaviours. Windows never load a button from an
//! ini themselves: at startup the game loads the six `[Button<id>]` sections of
//! `./scripts/buttons.ini` into a **template table**, and every button on screen is a clone
//! of one of them (`0x004C7A90`), re-textured with a caption and moved into place.
//!
//! A button is driven entirely by the scene container once registered: the container's
//! dispatcher sends it the mouse events, its event handler (`0x004C6B90`) tracks press and
//! release inside its rect and raises the clicked flag `+0xBC` on a left release (`-2`); the
//! owner polls [`Button::clicked`] each frame, which returns and clears the flag. The
//! trading office window polls its X button that way and closes itself.

use super::custom_window::{GameWindow, Widget};
use super::widget;

/// The template table: `TEMPLATE_COUNT` objects of `OBJECT_SIZE` bytes, filled at startup
/// by `0x00424DA4` from `[Button0]`..`[Button5]` of `./scripts/buttons.ini` (`0x0066DE5C`).
pub const TEMPLATE_TABLE: u32 = 0x006C_BE08;
pub const TEMPLATE_COUNT: *const u8 = 0x0066_DE5A as _;
pub const VTABLE: u32 = 0x0067_14C8;
const OBJECT_SIZE: usize = 0xE8;
/// `thiscall()`: the constructor - widget base with the class name it reads itself, the two
/// `CString`s at `+0xA8`, the look pointers zeroed, vtable, reset. Not `0x004C6A30`: that is
/// the destructor, which also writes the vtable first and then destroys the strings and runs
/// the base destructor chain - on a fresh buffer it corrupts the `CString` allocator.
const CTOR: u32 = 0x004C_6910;
/// `thiscall(this = template, dest)`, `ret 4`: copy the template into `dest` - position,
/// size, fresh copies of the four looks and the motion, both strings, type and flags.
const CLONE: u32 = 0x004C_7A90;
/// `thiscall(char*)`, `ret 4`: the caption (`CString` assign) and a redraw.
const SET_TEXT: u32 = 0x004C_7780;
/// `thiscall(type)`, `ret 4`: the behaviour, `+0xB8`; values above 2 are ignored.
const SET_KIND: u32 = 0x004C_7C30;
/// `thiscall() -> bool`: the clicked flag `+0xBC`, cleared on read. A repeat button also
/// reports while held, at the ini's delay.
const CLICKED: u32 = 0x004C_78B0;
/// Vtable `+0x120` = `0x004C7940`, `thiscall() -> bool`: the checked state of a check button
/// (the current look's `+0x10` flag).
const SLOT_IS_CHECKED: usize = 0x120;
/// Vtable `+0x124` = `0x004C7970`, `thiscall(bool)`, `ret 4`: set it, redrawing on change.
const SLOT_SET_CHECKED: usize = 0x124;

/// Our fields, past the game's object.
const FIELD_ATTACHED: u32 = 0xE8;
const BUFFER_SIZE: usize = 0xF0;

const _: () = assert!(FIELD_ATTACHED as usize >= OBJECT_SIZE);

/// The six looks of `buttons.ini`, by template id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonTemplate {
    /// `[Button0]`, 16 x 18: the round button - the lock checkbox's base, the arrows'.
    Round = 0,
    /// `[Button1]`, 32 x 18.
    Small = 1,
    /// `[Button2]`, 48 x 18.
    Medium = 2,
    /// `[Button3]`, 96 x 18.
    Large = 3,
    /// `[Button4]`, 128 x 18.
    Huge = 4,
    /// `[Button5]`, 32 x 18: the X that closes a window (TexID 16023).
    Close = 5,
}

/// The behaviours, `Type=` in the ini; the loader maps the names `normal`, `check`,
/// `permanent` (`0x006BF750`) to these values (`0x006714B8`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonKind {
    /// Reports a click on release.
    Normal = 0,
    /// Toggles its checked state on release; the pressed look while checked.
    Check = 1,
    /// Reports a click on press and keeps reporting while held.
    Repeat = 2,
}

#[repr(align(8))]
struct Buffer {
    _bytes: [u8; BUFFER_SIZE],
}

/// A handle to a button. Cheap to copy; the object lives for the rest of the process.
#[derive(Clone, Copy, Debug)]
pub struct Button {
    pub address: u32,
}

impl Button {
    /// Clone a template. `None` before the game has loaded its template table - it is filled
    /// during startup, after every mod's `start()`, so build buttons lazily (on first use).
    pub unsafe fn new(template: ButtonTemplate) -> Option<Self> {
        let id = template as u32;
        if id >= *TEMPLATE_COUNT as u32 {
            return None;
        }
        let source = TEMPLATE_TABLE + id * OBJECT_SIZE as u32;
        if *(source as *const u32) != VTABLE {
            return None;
        }
        let address = Box::leak(Box::new(Buffer { _bytes: [0; BUFFER_SIZE] })) as *mut Buffer as u32;
        let ctor: extern "thiscall" fn(u32) = std::mem::transmute(CTOR);
        ctor(address);
        let clone: extern "thiscall" fn(u32, u32) = std::mem::transmute(CLONE);
        clone(source, address);
        let button = Self { address };
        button.write(FIELD_ATTACHED, 0u32);
        Some(button)
    }

    /// The caption, latin1 (a NUL is appended).
    pub unsafe fn set_text(&self, text: &[u8]) {
        let mut bytes = text.to_vec();
        bytes.push(0);
        let set: extern "thiscall" fn(u32, *const u8) = std::mem::transmute(SET_TEXT);
        set(self.address, bytes.as_ptr());
    }

    pub unsafe fn set_kind(&self, kind: ButtonKind) {
        let set: extern "thiscall" fn(u32, u32) = std::mem::transmute(SET_KIND);
        set(self.address, kind as u32);
    }

    pub unsafe fn set_position(&self, x: i32, y: i32) {
        widget::set_position(self.address, x, y);
    }

    pub unsafe fn position(&self) -> (i32, i32) {
        widget::position(self.address)
    }

    /// Width and height, from the template.
    pub unsafe fn size(&self) -> (i32, i32) {
        widget::size(self.address)
    }

    pub unsafe fn show(&self, visible: bool) {
        widget::show(self.address, visible);
    }

    pub unsafe fn is_visible(&self) -> bool {
        widget::is_visible(self.address)
    }

    /// True once per click (release inside the button); also while held for a repeat button.
    pub unsafe fn clicked(&self) -> bool {
        let clicked: extern "thiscall" fn(u32) -> u8 = std::mem::transmute(CLICKED);
        clicked(self.address) != 0
    }

    pub unsafe fn is_checked(&self) -> bool {
        let checked: extern "thiscall" fn(u32) -> u8 = std::mem::transmute(widget::slot(self.address, SLOT_IS_CHECKED));
        checked(self.address) != 0
    }

    pub unsafe fn set_checked(&self, checked: bool) {
        let set: extern "thiscall" fn(u32, u32) = std::mem::transmute(widget::slot(self.address, SLOT_SET_CHECKED));
        set(self.address, checked as u32);
    }

    pub fn is_attached(&self) -> bool {
        unsafe { self.read::<u32>(FIELD_ATTACHED) != 0 }
    }

    /// Register with the top scene, for a button riding on a window that is not ours. Call
    /// after that window is in the container, so the button draws above it and gets the
    /// clicks.
    pub unsafe fn attach_to_root(&self) {
        if widget::add_to_root(self.address) {
            self.write(FIELD_ATTACHED, 1u32);
        }
    }

    pub unsafe fn detach(&self) {
        widget::remove_from_root(self.address);
        self.write(FIELD_ATTACHED, 0u32);
    }

    unsafe fn read<T: Copy>(&self, offset: u32) -> T {
        *((self.address + offset) as *const T)
    }
    unsafe fn write<T>(&self, offset: u32, value: T) {
        *((self.address + offset) as *mut T) = value;
    }
}

impl Widget for Button {
    unsafe fn attach(&mut self, _window: &GameWindow, _root: u32) {
        Button::attach_to_root(self);
    }

    unsafe fn detach(&mut self) {
        Button::detach(self);
    }
}
