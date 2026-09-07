//! The slots every game widget shares - the common base class (constructor `0x004B15F0`,
//! vtable `0x006708D0`) that buttons, images, scrollbars and the windows derive from - and
//! membership of the scene container, which is what makes a widget live: the container
//! draws and updates its children each frame and routes the mouse to the topmost one under
//! the cursor. Children are drawn in registration order, so a widget meant to sit on a
//! window is added after it.
//!
//! Positions are screen coordinates for a direct child of the scene (its entry offsets are
//! zero). Sizes come from the class's ini section and are copied along when a widget is
//! cloned.
//!
//! # Drawing on a game widget: check the origin first
//!
//! A widget's draw slot ([SLOT_DRAW]) is called **more than once per frame**, and only one
//! of those passes may be drawn into. The pass to use has its two origin arguments at
//! `0,0`, so the widget's own `+0x14`/`+0x18` are already the screen position. Another pass
//! passes the **negative** of those fields, rendering the widget at `(0,0)` in a local
//! space of its own.
//!
//! This matters because the drawing helpers - `ui_render_text_at`,
//! `ddraw_fill_solid_rect` and friends - take screen coordinates and are **not clipped to
//! the widget**. Drawn during the local pass they follow it into whatever surface it
//! targets and land somewhere unrelated on screen, and because only the widget's own area
//! is repainted each frame, the result stays visible until something else happens to redraw
//! that region. Measured on the scrollmap's ship panel, whose `+0x14`/`+0x18` are
//! `1404,353`: the good pass gave origin `0,0`, the other `-1404,-353`, and drawing in the
//! second put text over the minimap.
//!
//! So a hook that paints on a widget must return early unless both origin arguments are
//! zero. Two further traps in the same place: the slot belongs to the **class**, so the
//! hook fires for every instance and has to compare the `this` it is given against the
//! object it means to decorate; and a panel may be drawn repeatedly with different
//! internal state, so anything read out of it can differ between calls of the same frame.

use super::custom_window::{root_container, CONTAINER_ADD, CONTAINER_REMOVE};

/// `thiscall(x, y, z)`, `ret 0xC` (base `0x00402550`): writes `+0x14`/`+0x18`. The game's
/// windows pass [`CHILD_Z`] for `z`.
/// `event(point*, type)`, `ret 8`: the root container's input dispatcher calls it on the
/// topmost child under the cursor. Type `1` is a press, `-2` a left click.
pub const SLOT_EVENT: usize = 0x18;
/// `close()`, run on every way of leaving the window.
pub const SLOT_CLOSE: usize = 0x118;
/// `open()`, run when the window is put on screen.
pub const SLOT_OPEN: usize = 0x120;
/// `draw(context, x, y, z)`, `ret 0x10`: the scene container calls it each frame the widget
/// meets the dirty region.
pub const SLOT_DRAW: usize = 0x9C;
pub const SLOT_SET_POSITION: usize = 0x64;
/// `thiscall(out*)`, `ret 4` (base `0x00402470`): copies `+0x2C`, `+0x30`, `+0x34` - width,
/// height, depth.
pub const SLOT_SIZE: usize = 0x40;
/// `thiscall() -> bool` (base `0x004B2200`): the visible byte `+0x48`.
pub const SLOT_IS_VISIBLE: usize = 0xC8;
/// `thiscall(bool) -> bool previous`, `ret 4` (base `0x004B21A0`): show or hide, marking the
/// widget dirty when the state changes.
pub const SLOT_SHOW: usize = 0xCC;
/// The z every game window gives its children.
pub const CHILD_Z: i32 = 0x64;

const FIELD_X: u32 = 0x14;
const FIELD_Y: u32 = 0x18;

/// A virtual method's address.
pub unsafe fn slot(widget: u32, slot: usize) -> u32 {
    *((*(widget as *const u32) + slot as u32) as *const u32)
}

pub unsafe fn set_position(widget: u32, x: i32, y: i32) {
    let set: extern "thiscall" fn(u32, i32, i32, i32) = std::mem::transmute(slot(widget, SLOT_SET_POSITION));
    set(widget, x, y, CHILD_Z);
}

pub unsafe fn position(widget: u32) -> (i32, i32) {
    (*((widget + FIELD_X) as *const i32), *((widget + FIELD_Y) as *const i32))
}

/// Width and height.
pub unsafe fn size(widget: u32) -> (i32, i32) {
    let mut out = [0i32; 3];
    let get: extern "thiscall" fn(u32, *mut i32) = std::mem::transmute(slot(widget, SLOT_SIZE));
    get(widget, out.as_mut_ptr());
    (out[0], out[1])
}

/// `thiscall(width, height, depth) -> bool`, `ret 0xC` (base `0x004024B0`): stores the three
/// into `+0x2C`, `+0x30`, `+0x34`. The UI startup routine sizes every building window
/// through it with hard-coded `425 x 510` (`0x00426D98`); nothing resizes them afterwards.
pub const SLOT_SET_SIZE: usize = 0x4C;

/// Depth, the third value the size setter stores.
const FIELD_DEPTH: u32 = 0x34;

/// Resize a widget, keeping its depth.
pub unsafe fn set_size(widget: u32, width: i32, height: i32) {
    let depth = *((widget + FIELD_DEPTH) as *const i32);
    let set: extern "thiscall" fn(u32, i32, i32, i32) -> u8 = std::mem::transmute(slot(widget, SLOT_SET_SIZE));
    set(widget, width, height, depth);
}

pub unsafe fn show(widget: u32, visible: bool) {
    let show: extern "thiscall" fn(u32, u32) -> u8 = std::mem::transmute(slot(widget, SLOT_SHOW));
    show(widget, visible as u32);
}

/// `thiscall(bool)`, `ret 4` (base `0x004B17F0`): writes the enabled byte `+0x41` that the
/// event handlers test before dispatching, and on enabling asks `+0x28`. The scene switch
/// between the scrollmap and the local map disables the one it hides and enables the one it
/// shows (`0x0044A1D7`/`0x0044A1F4`).
pub const SLOT_SET_ENABLED: usize = 0x34;

pub unsafe fn set_enabled(widget: u32, enabled: bool) {
    let set: extern "thiscall" fn(u32, u32) = std::mem::transmute(slot(widget, SLOT_SET_ENABLED));
    set(widget, enabled as u32);
}

pub unsafe fn is_visible(widget: u32) -> bool {
    let visible: extern "thiscall" fn(u32) -> u8 = std::mem::transmute(slot(widget, SLOT_IS_VISIBLE));
    visible(widget) != 0
}

/// Register the widget with the scene on top of the window stack. False when no scene is up.
pub unsafe fn add_to_root(widget: u32) -> bool {
    let root = root_container();
    if root == 0 {
        return false;
    }
    let add: extern "thiscall" fn(u32, u32) = std::mem::transmute(CONTAINER_ADD);
    add(root, widget);
    true
}

/// Take the widget out of the top scene; a no-op when it is not a child.
pub unsafe fn remove_from_root(widget: u32) {
    let root = root_container();
    if root == 0 {
        return;
    }
    let remove: extern "thiscall" fn(u32, u32) = std::mem::transmute(CONTAINER_REMOVE);
    remove(root, widget);
}
