//! Windows of our own inside the game's window system.
//!
//! A [`GameWindow`] is a real game window object: a buffer run through the shared window
//! base constructor (`0x0041C320`, the `CDialogBG` parchment widget every building window
//! derives from) that gets **its own copy of the base vtable** with five slots redirected
//! to trampolines, which call back into a Rust [`WindowContent`]. Once opened the game
//! drives it exactly like the ship overview:
//!
//! - The **scene on top of the window stack** (`0x006DA5F0`; `0x004B9730` returns it) is
//!   the widget container that calls every child's update `+0xF4` and draw `+0x9C` each
//!   frame - the scrollmap or the town view, whichever is showing. A window
//!   and its sub-widgets are all *direct* children of that one flat container; later
//!   children are drawn later and are the topmost hit under the cursor, so the window is
//!   added first and its [`Widget`]s after it, in order.
//! - The **open-window list** `0x006CC3B4` (joined through the base show `0x00462390`)
//!   calls the per-frame slot `+0x124` from both scene draws, and ESC / right-click /
//!   opening something else close the topmost window through `+0x11C` (may I close) and
//!   `+0x118` (close). Dismissal therefore costs nothing.
//! - The **renderer repaints dirty rectangles only**, and every draw is clipped to them:
//!   content that changes must call [`GameWindow::invalidate`], as the game's own windows
//!   do through `0x004B9650`.
//! - The **parchment painter** `0x0041CAA0` tiles the shared `[DialogBG0]` resource in
//!   whole 32-px tiles, after `0x0041C5A0` has built the window's tile map (`+0xA0`). Sizes
//!   are snapped up to the tile so no strip is left unpainted.
//!
//! Base-class layout used here: `+0x14`/`+0x18` x/y (screen coordinates for a root child),
//! `+0x2C`/`+0x30` w/h, `+0x48` visible byte, `+0x94` shadow flag, `+0xA0` tile map. The
//! base fields end below `+0xD0` (the ship overview's first own member); this object is
//! `0x100` bytes and keeps its Rust state pointer at `+0xF0`.
//!
//! Beware the constructor/destructor pair: `0x00430FE0` and `0x00460A70` in the scrollbar
//! family are destructors that also write their vtable first. Vtable slot 0 (the deleting
//! destructor) tells them apart.

use std::ffi::c_void;

use crate::data::{fill_p3_string, render_window_title, screen_rectangle::Rect};

/// The window base class's vtable (`CDialogBG`), 74 entries.
pub const WINDOW_BASE_VTABLE: u32 = 0x0066_BC90;
pub const WINDOW_BASE_VTABLE_LEN: usize = 74;
/// `thiscall()`: constructs the widget base (name `CDialogBG`), then writes the base vtable.
const WINDOW_BASE_CTOR: u32 = 0x0041_C320;
/// `thiscall(w, h, shadow_flag)`, `ret 0xC`: builds the parchment tile map - `+0x98/+0x9C`
/// from the size, `+0x94 = flag`, `+0xA0 = new[]` of random tile variants. The painter
/// draws nothing while `+0xA0` is 0; the base close frees it.
const PARCHMENT_PREPARE: u32 = 0x0041_C5A0;
/// `thiscall(rect*)`, `ret 4`: paints the prepared parchment into the screen rect.
const PARCHMENT_PAINT: u32 = 0x0041_CAA0;
/// `thiscall`: append to the open-window list and re-post the mouse (the base show).
const LIST_SHOW: u32 = 0x0046_2390;
/// `thiscall`: invalidate the window rect and remove it from the open-window list.
const LIST_REMOVE: u32 = 0x0046_2320;
/// `cdecl(force)`: close every open window, honouring their `+0x11C` veto unless forced.
const CLOSE_ALL_OPEN_WINDOWS: u32 = 0x0046_2280;
/// The window stack; the scene on top of it is the widget container everything else lives in.
pub const CONTAINER_STACK: u32 = 0x006D_A5F0;
/// `thiscall(stack) -> container*`: the scene on top of the stack, or 0 when it is empty.
const CONTAINER_ROOT: u32 = 0x004B_9730;
/// `thiscall(child)`, `ret 4`: append a child (offset from its `+0x84` rect).
pub(crate) const CONTAINER_ADD: u32 = 0x004B_4E30;
/// `thiscall(child)`, `ret 4`: remove a child; a no-op when absent.
pub(crate) const CONTAINER_REMOVE: u32 = 0x004B_4EB0;
/// `cdecl(rect*) -> bool`: begin a draw; 0 when the rect misses the dirty region.
const RENDER_BEGIN_RECT: u32 = 0x004B_B7C0;
/// `cdecl(context)`: the render context handed down the widget tree as the draw's first
/// stack argument.
const RENDER_SET_CONTEXT: u32 = 0x004B_B9B0;
/// `cdecl(texture)`: `-1` before the parchment.
const RENDER_SET_TEXTURE: u32 = 0x004B_B870;
/// `cdecl(x0, y0, x1, y1)`: declares the window's opaque backdrop, posted from `+0x124`.
const RENDER_BACKDROP: u32 = 0x004B_BAA0;
const SHADOW_SIZE: *const i32 = 0x006C_BB08 as _;
const SHADOW_EXTRA: *const i32 = 0x006C_BAEC as _;
/// `thiscall(stack, rect*)`, `ret 4`: mark a screen rect dirty.
const INVALIDATE_RECT: u32 = 0x004B_9650;

/// The parchment tile; window sizes are multiples of it.
pub const TILE_SIZE: i32 = 32;

const SLOT_LOAD: usize = 0x8;
const SLOT_DRAW: usize = 0x9C;
const SLOT_UPDATE: usize = 0xF4;
const SLOT_CLOSE: usize = 0x118;
const SLOT_FRAME: usize = 0x124;
const FIELD_X: u32 = 0x14;
const FIELD_Y: u32 = 0x18;
const FIELD_W: u32 = 0x2C;
const FIELD_H: u32 = 0x30;
const FIELD_VISIBLE: u32 = 0x48;
const FIELD_SHADOW: u32 = 0x94;
/// Our slot in the object, past every base field.
const FIELD_STATE: u32 = 0xF0;
const OBJECT_SIZE: usize = 0x100;

/// What a window draws and does. The parchment (and the title, if set) are already painted
/// when `draw` runs.
pub trait WindowContent {
    /// Called each frame the window's rect meets the renderer's dirty region. `ctx` carries
    /// the window's screen rect; draw with `ui_render_text_at` and friends.
    fn draw(&mut self, window: &GameWindow, ctx: &DrawContext);
    /// Called each frame while open, before the draws.
    fn update(&mut self, _window: &GameWindow) {}
    /// Called when the window closes, by whichever path.
    fn on_close(&mut self, _window: &GameWindow) {}
}

/// A game widget that lives in the root container while the window is open - registered
/// right after the window, in the order added, so that it draws above it.
pub trait Widget {
    /// Register with the root container (and place yourself in screen coordinates).
    unsafe fn attach(&mut self, window: &GameWindow, root: u32);
    /// Leave the root container.
    unsafe fn detach(&mut self);
    /// Per frame while open.
    unsafe fn update(&mut self, _window: &GameWindow) {}
}

/// The window's screen rect at draw time (its position plus the container's offsets).
#[derive(Clone, Copy, Debug)]
pub struct DrawContext {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub render_context: u32,
}

struct State {
    content: Box<dyn WindowContent>,
    widgets: Vec<Box<dyn Widget>>,
    open: bool,
    /// The title, NUL-terminated latin1. A game `CString` is built from it on every draw,
    /// because the title renderer takes its string **by value and destroys it** (MSVC
    /// callee-destroys: `0x00420DBF` runs `~CString` `0x0064F253` on the argument). Passing
    /// one string twice frees it twice and corrupts the heap.
    title: Option<Vec<u8>>,
}

#[repr(align(8))]
struct ObjectBuffer {
    _bytes: [u8; OBJECT_SIZE],
}

/// A handle to one of our windows. Cheap to copy; the object itself lives for the rest of
/// the process, like every game window.
#[derive(Clone, Copy, Debug)]
pub struct GameWindow {
    pub address: u32,
}

impl GameWindow {
    /// Build a window at `x`,`y` (screen coordinates) of at least `width` x `height`,
    /// snapped up to whole parchment tiles. Nothing is shown until [`GameWindow::open`].
    pub unsafe fn new(x: i32, y: i32, width: i32, height: i32, content: Box<dyn WindowContent>) -> Self {
        let width = snap_to_tile(width);
        let height = snap_to_tile(height);
        // The game rounds its windows' x up to a multiple of 4 (ship overview, 0x0047551C).
        let x = (x + 3) & !3;

        let object = Box::leak(Box::new(ObjectBuffer { _bytes: [0; OBJECT_SIZE] })) as *mut ObjectBuffer as u32;
        let vtable: &'static mut [u32; WINDOW_BASE_VTABLE_LEN] = Box::leak(Box::new([0; WINDOW_BASE_VTABLE_LEN]));
        std::ptr::copy_nonoverlapping(WINDOW_BASE_VTABLE as *const u32, vtable.as_mut_ptr(), WINDOW_BASE_VTABLE_LEN);
        vtable[SLOT_LOAD / 4] = load_trampoline as usize as u32;
        vtable[SLOT_DRAW / 4] = draw_trampoline as usize as u32;
        vtable[SLOT_UPDATE / 4] = update_trampoline as usize as u32;
        vtable[SLOT_CLOSE / 4] = close_trampoline as usize as u32;
        vtable[SLOT_FRAME / 4] = frame_trampoline as usize as u32;

        // The widget base init dispatches through `[this]` after writing its own vtable,
        // and the base constructor ends by writing the base table - so ours goes in after.
        let ctor: extern "thiscall" fn(u32) = std::mem::transmute(WINDOW_BASE_CTOR);
        ctor(object);
        *(object as *mut u32) = vtable.as_ptr() as u32;

        let window = Self { address: object };
        window.write(FIELD_X, x);
        window.write(FIELD_Y, y);
        window.write(FIELD_W, width);
        window.write(FIELD_H, height);
        window.write::<u8>(FIELD_VISIBLE, 1);
        let prepare: extern "thiscall" fn(u32, i32, i32, u32) = std::mem::transmute(PARCHMENT_PREPARE);
        prepare(object, width, height, 0);

        let state = Box::new(State { content, widgets: Vec::new(), open: false, title: None });
        window.write(FIELD_STATE, Box::into_raw(state) as u32);
        window
    }

    /// Add a sub-widget. Widgets are attached after the window on every open, in this
    /// order, and detached on close.
    pub unsafe fn add_widget(&self, widget: Box<dyn Widget>) {
        self.state().widgets.push(widget);
    }

    /// A title drawn by the game's own window-title renderer (latin1 bytes, no NUL needed).
    pub unsafe fn set_title(&self, title: &[u8]) {
        let mut bytes = title.to_vec();
        bytes.push(0);
        self.state().title = Some(bytes);
    }

    /// Close every other window, join the root container and the open-window list.
    /// Returns false when there is no scene root to join (no scene up yet).
    pub unsafe fn open(&self) -> bool {
        if self.is_open() {
            return true;
        }
        let close_all: extern "cdecl" fn(u32) -> u32 = std::mem::transmute(CLOSE_ALL_OPEN_WINDOWS);
        close_all(0);
        let root = root_container();
        if root == 0 {
            return false;
        }
        let remove: extern "thiscall" fn(u32, u32) = std::mem::transmute(CONTAINER_REMOVE);
        let add: extern "thiscall" fn(u32, u32) = std::mem::transmute(CONTAINER_ADD);
        remove(root, self.address);
        add(root, self.address);
        let window = *self;
        for widget in &mut self.state().widgets {
            widget.attach(&window, root);
        }
        let show: extern "thiscall" fn(u32) = std::mem::transmute(LIST_SHOW);
        show(self.address);
        self.state().open = true;
        true
    }

    /// Leave the open-window list and the root container, widgets first. Also what the
    /// game calls through `+0x118` on ESC or right-click.
    pub unsafe fn close(&self) {
        close_trampoline(self.address);
    }

    pub fn is_open(&self) -> bool {
        unsafe { self.state().open }
    }

    pub fn x(&self) -> i32 {
        unsafe { self.read(FIELD_X) }
    }
    pub fn y(&self) -> i32 {
        unsafe { self.read(FIELD_Y) }
    }
    pub fn width(&self) -> i32 {
        unsafe { self.read(FIELD_W) }
    }
    pub fn height(&self) -> i32 {
        unsafe { self.read(FIELD_H) }
    }
    /// The window's screen rect (position plus size).
    pub fn rect(&self) -> Rect {
        let (x, y) = (self.x(), self.y());
        Rect { left: x, top: y, right: x + self.width(), bottom: y + self.height() }
    }

    /// Mark the whole window dirty so the next frame repaints it. Content that changes
    /// (a scrolled list, a changed value) must call this, or it repaints in slivers.
    pub unsafe fn invalidate(&self) {
        invalidate_rect(&self.rect());
    }

    unsafe fn state(&self) -> &mut State {
        &mut *(self.read::<u32>(FIELD_STATE) as *mut State)
    }
    unsafe fn read<T: Copy>(&self, offset: u32) -> T {
        *((self.address + offset) as *const T)
    }
    unsafe fn write<T>(&self, offset: u32, value: T) {
        *((self.address + offset) as *mut T) = value;
    }
}

/// The scene on top of the window stack - the container windows live in - or 0 when none is up.
pub unsafe fn root_container() -> u32 {
    let root_of: extern "thiscall" fn(u32) -> u32 = std::mem::transmute(CONTAINER_ROOT);
    root_of(CONTAINER_STACK)
}

/// Mark a screen rect dirty on the container stack.
pub unsafe fn invalidate_rect(rect: &Rect) {
    let invalidate: extern "thiscall" fn(u32, *const Rect) = std::mem::transmute(INVALIDATE_RECT);
    invalidate(CONTAINER_STACK, rect);
}

fn snap_to_tile(size: i32) -> i32 {
    (size.max(TILE_SIZE) + TILE_SIZE - 1) / TILE_SIZE * TILE_SIZE
}

/// Slot `+0x8`, a pure virtual in the base (the derived windows load their ini section
/// here). Nobody calls it on us; a stub beats `_purecall`.
unsafe extern "thiscall" fn load_trampoline(_this: u32, _ini: u32, _id: u32) {}

/// Slot `+0x9C`. The container passes its own first argument (the render context) and
/// then the child entry's `{x, y, z}` offsets - zero for a root child - which the ship
/// overview adds to `+0x14`/`+0x18` for its screen rect, as we do.
unsafe extern "thiscall" fn draw_trampoline(this: u32, render_context: u32, offset_x: i32, offset_y: i32, _offset_z: u32) {
    let window = GameWindow { address: this };
    if window.read::<u8>(FIELD_VISIBLE) == 0 {
        return;
    }
    let x = window.x() + offset_x;
    let y = window.y() + offset_y;
    let (width, height) = (window.width(), window.height());
    let rect = Rect { left: x, top: y, right: x + width, bottom: y + height };
    let begin: extern "cdecl" fn(*const Rect) -> u32 = std::mem::transmute(RENDER_BEGIN_RECT);
    if begin(&rect) == 0 {
        return;
    }
    let set_context: extern "cdecl" fn(u32) = std::mem::transmute(RENDER_SET_CONTEXT);
    let set_texture: extern "cdecl" fn(u32) = std::mem::transmute(RENDER_SET_TEXTURE);
    set_context(render_context);
    set_texture(u32::MAX);
    let paint: extern "thiscall" fn(u32, *const Rect) = std::mem::transmute(PARCHMENT_PAINT);
    paint(this, &rect);

    let state = window.state();
    if let Some(title) = &state.title {
        let mut p3_string: u32 = 0;
        fill_p3_string((&mut p3_string) as *mut u32 as *const c_void, title);
        render_window_title(p3_string as *const c_void, this as *const c_void);
    }
    let ctx = DrawContext { x, y, width, height, render_context };
    state.content.draw(&window, &ctx);
}

/// Slot `+0xF4`, from the container each frame while we are a child.
unsafe extern "thiscall" fn update_trampoline(this: u32) {
    let window = GameWindow { address: this };
    let state = window.state();
    for widget in &mut state.widgets {
        widget.update(&window);
    }
    state.content.update(&window);
}

/// Slot `+0x124`, from the open-window driver: the ship overview's `0x004749C0`, posting
/// the backdrop rect inset by the shadow metrics.
unsafe extern "thiscall" fn frame_trampoline(this: u32) {
    let window = GameWindow { address: this };
    let (x, y, w, h) = (window.x(), window.y(), window.width(), window.height());
    let s = *SHADOW_SIZE;
    let f = if window.read::<u8>(FIELD_SHADOW) != 0 { *SHADOW_EXTRA } else { 0 };
    let backdrop: extern "cdecl" fn(i32, i32, i32, i32) = std::mem::transmute(RENDER_BACKDROP);
    backdrop(x + s + f, y + s / 2, x + w - s - f, y + h - s);
}

/// Slot `+0x118`. The base close would also free the tile map at `+0xA0`; ours keeps it
/// for the next open.
unsafe extern "thiscall" fn close_trampoline(this: u32) {
    let window = GameWindow { address: this };
    let state = window.state();
    if !state.open {
        return;
    }
    let list_remove: extern "thiscall" fn(u32) = std::mem::transmute(LIST_REMOVE);
    list_remove(this);
    for widget in state.widgets.iter_mut().rev() {
        widget.detach();
    }
    let root = root_container();
    if root != 0 {
        let remove: extern "thiscall" fn(u32, u32) = std::mem::transmute(CONTAINER_REMOVE);
        remove(root, this);
    }
    state.open = false;
    state.content.on_close(&window);
}
