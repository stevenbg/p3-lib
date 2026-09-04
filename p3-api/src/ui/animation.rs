//! `C2DAnimation` - the game's image widget: one texture frame, or a timed sequence of them,
//! that draws itself and **ignores input** (its event slot `+0x18` = `0x004AD890` is a bare
//! `ret 8`). That is what lets the game stack one on a button: the trading office's lock
//! checkbox is a round button with this class's checkmark image registered right after it.
//!
//! A section `[ANIM<id>]` describes it: `Count`, `FrameCount<n>`, and `Frame<n>` entries of
//! `TexID frame x y _ w h _`, plus the timers. The four looks of a button are objects of
//! this class too.

use crate::data::fill_p3_string;

use super::custom_window::{GameWindow, Widget};
use super::widget;

/// `thiscall()`: widget base (name `C2DAnimation`), the sequence object at `+0xA8`, vtable,
/// reset.
const CTOR: u32 = 0x004A_CDD0;
/// Vtable `+0x8`, `thiscall(CString by value, id)`, `ret 8`: load `[ANIM<id>]` from the ini.
/// Consumes the string.
const LOAD: u32 = 0x004A_CF90;
pub const VTABLE: u32 = 0x0067_02F0;
const OBJECT_SIZE: usize = 0xD8;
/// The shared empty-string block; `+0xC` is the `CString` value for "".
const NIL_STRING_HEADER: *const u32 = 0x006C_7CD0 as _;

/// The trading office's lock checkmark: `[ANIM12]` of `BuildingParchment.ini` - TexID 20008,
/// frame 0, 24 x 16 - drawn at `(button.x - 4, button.y)` over a 16 x 18 round button.
pub const LOCK_CHECKMARK_INI: &[u8] = b"./scripts/BuildingParchment.ini\0";
pub const LOCK_CHECKMARK_ID: u32 = 12;
pub const LOCK_CHECKMARK_X_OFFSET: i32 = -4;

/// Our fields, past the game's object.
const FIELD_ATTACHED: u32 = 0xD8;
const BUFFER_SIZE: usize = 0xE0;

const _: () = assert!(FIELD_ATTACHED as usize >= OBJECT_SIZE);

#[repr(align(8))]
struct Buffer {
    _bytes: [u8; BUFFER_SIZE],
}

/// A handle to an image widget. Cheap to copy; the object lives for the rest of the process.
#[derive(Clone, Copy, Debug)]
pub struct Animation2D {
    pub address: u32,
}

impl Animation2D {
    /// Construct and load `[ANIM<id>]` from `ini` (a NUL-terminated `./scripts/...` path).
    /// Hidden and unplaced until [`Animation2D::set_position`] and [`Animation2D::show`].
    pub unsafe fn new(ini: &[u8], id: u32) -> Self {
        let address = Box::leak(Box::new(Buffer { _bytes: [0; BUFFER_SIZE] })) as *mut Buffer as u32;
        let ctor: extern "thiscall" fn(u32) = std::mem::transmute(CTOR);
        ctor(address);
        // The loader takes the string by value and releases it: hand it a fresh one.
        let mut ini_string = *NIL_STRING_HEADER + 0xC;
        fill_p3_string((&mut ini_string) as *mut u32 as _, ini);
        let load: extern "thiscall" fn(u32, u32, u32) = std::mem::transmute(LOAD);
        load(address, ini_string, id);
        let image = Self { address };
        image.write(FIELD_ATTACHED, 0u32);
        image
    }

    pub unsafe fn set_position(&self, x: i32, y: i32) {
        widget::set_position(self.address, x, y);
    }

    pub unsafe fn position(&self) -> (i32, i32) {
        widget::position(self.address)
    }

    /// Width and height, from the frame.
    pub unsafe fn size(&self) -> (i32, i32) {
        widget::size(self.address)
    }

    pub unsafe fn show(&self, visible: bool) {
        widget::show(self.address, visible);
    }

    pub unsafe fn is_visible(&self) -> bool {
        widget::is_visible(self.address)
    }

    pub fn is_attached(&self) -> bool {
        unsafe { self.read::<u32>(FIELD_ATTACHED) != 0 }
    }

    /// Register with the top scene, for an image riding on a window that is not ours. Call
    /// after that window (and whatever the image should cover) is in the container.
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

impl Widget for Animation2D {
    unsafe fn attach(&mut self, _window: &GameWindow, _root: u32) {
        Animation2D::attach_to_root(self);
    }

    unsafe fn detach(&mut self) {
        Animation2D::detach(self);
    }
}
