//! The game's list scrollbar - `CP2Scrollbar` inside its list controller - as a
//! [`Widget`] for a [`GameWindow`].
//!
//! The controller (ctor `0x00430F90`, vtable `0x0066CFC8`, `0x3B4` bytes) embeds the bar at
//! `+0x8` and turns its position into a first visible row (`+0x3A4`). The bar (vtable
//! `0x0066E120`) owns three buttons (`+0x94` up, `+0x17C` down, `+0x264` thumb) loaded from
//! `[P2Scrollbar0]` in `parchment.ini` (inside `p2arch0_eng.cpr`); its position is `+0x36C`,
//! range (visible rows) `+0x37C`, step `+0x374`, scrollable flag `+0x360`.
//!
//! Everything is driven by the game once the bar is a root-container child: the container
//! runs its update (thumb drag) and draw, and the container's mouse-wheel handler
//! (`0x004B6A90`, reached from the MFC `WM_MOUSEWHEEL` map) hands wheel notches to the bar
//! whose catchment rect `+0x380` contains the cursor. That rect is the *area* given to
//! `set_rect`, so hand it the whole list, not the bar column.

use crate::data::{fill_p3_string, screen_rectangle::Rect};

use super::custom_window::{GameWindow, Widget};

/// `thiscall()`: constructs the bar at `+0x8`, the vtable, zeroes `+0x398..+0x3B0`.
const CONTROLLER_CTOR: u32 = 0x0043_0F90;
/// `thiscall(n)`, `ret 4`: the item count. Shows the bar when `n` exceeds the visible rows
/// and hides it otherwise, and **only when the shown state or the count changes** pushes
/// the total into the bar and re-reads the position - so the bar is hidden before every
/// call, as the ship overview does (`0x004757FD`), or a re-placed bar keeps a stale position.
const CONTROLLER_SET_COUNT: u32 = 0x0043_11B0;
/// `thiscall() -> bool moved`: read the bar, apply button clicks, recompute the rows.
const CONTROLLER_POLL: u32 = 0x0043_1260;
const CONTROLLER_BAR: u32 = 0x8;
const CONTROLLER_FIRST_ROW: u32 = 0x3A4;
const CONTROLLER_SIZE: usize = 0x3B4;
/// Vtable `+0x8`, `thiscall(CString by value, id)`, `ret 8`: load `[P2Scrollbar<id>]`;
/// consumes the string.
const SCROLLBAR_LOAD_INI: u32 = 0x0046_0B70;
/// `thiscall(area*, total, step, container)`, `ret 0x10`. Places the bar against the
/// area's right edge (`+0x14 = right - button width`), takes the range from its height,
/// copies the area to the wheel catchment `+0x380`, and registers the bar and its three
/// buttons in the container (0 = root), removing them first if present.
const SCROLLBAR_SET_RECT: u32 = 0x0046_0EF0;
/// `thiscall()`: remove the bar and its three buttons from the root container.
const SCROLLBAR_DETACH: u32 = 0x0046_1480;
/// `thiscall()`: release the thumb button; called before re-placing a bar.
const SCROLLBAR_RELEASE_THUMB: u32 = 0x0046_1FD0;
const SCROLLBAR_SHOW_SLOT: u32 = 0xCC;
const SCROLLBAR_RANGE: u32 = 0x37C;
const SCROLLBAR_INI: &[u8] = b"./scripts/parchment.ini\0";
const SCROLLBAR_INI_ID: u32 = 0;
/// The shared empty-string block; `+0xC` is the `CString` value for "".
const NIL_STRING_HEADER: *const u32 = 0x006C_7CD0 as _;

/// Our fields, past the controller.
const FIELD_AREA: u32 = 0x3C0;
const FIELD_ROW_HEIGHT: u32 = 0x3D0;
const FIELD_COUNT: u32 = 0x3D4;
const FIELD_ATTACHED: u32 = 0x3D8;
const BUFFER_SIZE: usize = 0x3E0;

// Our fields must lie past the game's object.
const _: () = assert!(FIELD_AREA as usize >= CONTROLLER_SIZE);

#[repr(align(8))]
struct Buffer {
    _bytes: [u8; BUFFER_SIZE],
}

/// A handle to a list scrollbar. Cheap to copy: the content keeps one to read the first
/// row, the window keeps one as a [`Widget`]. The object lives for the rest of the process.
#[derive(Clone, Copy, Debug)]
pub struct ScrollList {
    pub address: u32,
}

impl ScrollList {
    /// Construct and load the parchment scrollbar. Place it with [`ScrollList::set_area`]
    /// and give it a count before the window opens.
    pub unsafe fn new() -> Self {
        let address = Box::leak(Box::new(Buffer { _bytes: [0; BUFFER_SIZE] })) as *mut Buffer as u32;
        let ctor: extern "thiscall" fn(u32) = std::mem::transmute(CONTROLLER_CTOR);
        ctor(address);
        let list = Self { address };
        // The loader takes the string by value and releases it: hand it a fresh one.
        let mut ini = *NIL_STRING_HEADER + 0xC;
        fill_p3_string((&mut ini) as *mut u32 as _, SCROLLBAR_INI);
        let load: extern "thiscall" fn(u32, u32, u32) = std::mem::transmute(SCROLLBAR_LOAD_INI);
        load(list.bar(), ini, SCROLLBAR_INI_ID);
        list.write(FIELD_AREA, Rect { left: 0, top: 0, right: 0, bottom: 0 });
        list.write(FIELD_ROW_HEIGHT, 16i32);
        list.write(FIELD_COUNT, 0u32);
        list.write(FIELD_ATTACHED, 0u32);
        list
    }

    /// The list area in screen coordinates and the row height. The bar sits against the
    /// area's right edge; the area's height over the row height is the visible row count;
    /// the mouse wheel works anywhere inside the area. Takes effect on the next attach.
    pub unsafe fn set_area(&self, area: Rect, row_height: i32) {
        self.write(FIELD_AREA, area);
        self.write(FIELD_ROW_HEIGHT, row_height);
    }

    /// The number of rows. Applied at once while attached.
    pub unsafe fn set_count(&self, count: u32) {
        self.write(FIELD_COUNT, count);
        if self.read::<u32>(FIELD_ATTACHED) != 0 {
            self.apply_count();
        }
    }

    /// The first visible row, as the bar stands now.
    pub fn first_row(&self) -> i32 {
        unsafe { self.read::<i32>(CONTROLLER_FIRST_ROW).max(0) }
    }

    /// How many rows the area shows.
    pub fn visible_rows(&self) -> i32 {
        unsafe { *((self.bar() + SCROLLBAR_RANGE) as *const i32) }
    }

    /// Read the bar and apply pending clicks. True when the position moved this frame.
    pub unsafe fn poll(&self) -> bool {
        let poll: extern "thiscall" fn(u32) -> u8 = std::mem::transmute(CONTROLLER_POLL);
        poll(self.address) != 0
    }

    fn bar(&self) -> u32 {
        self.address + CONTROLLER_BAR
    }

    unsafe fn apply_count(&self) {
        // Hidden first, so set_count sees a state change and resyncs (see its doc).
        let show: extern "thiscall" fn(u32, u32) = std::mem::transmute(*((*(self.bar() as *const u32) + SCROLLBAR_SHOW_SLOT) as *const u32));
        show(self.bar(), 0);
        let set_count: extern "thiscall" fn(u32, u32) = std::mem::transmute(CONTROLLER_SET_COUNT);
        set_count(self.address, self.read(FIELD_COUNT));
    }

    unsafe fn read<T: Copy>(&self, offset: u32) -> T {
        *((self.address + offset) as *const T)
    }
    unsafe fn write<T>(&self, offset: u32, value: T) {
        *((self.address + offset) as *mut T) = value;
    }
}

impl Widget for ScrollList {
    /// The ship overview's per-open order (`0x004757E0`..): release the thumb, place (which
    /// registers bar and buttons in the container), hide, then the count.
    unsafe fn attach(&mut self, _window: &GameWindow, root: u32) {
        let release_thumb: extern "thiscall" fn(u32) = std::mem::transmute(SCROLLBAR_RELEASE_THUMB);
        release_thumb(self.bar());
        let area: Rect = self.read(FIELD_AREA);
        let set_rect: extern "thiscall" fn(u32, *const Rect, i32, i32, u32) = std::mem::transmute(SCROLLBAR_SET_RECT);
        set_rect(self.bar(), &area, 0, self.read(FIELD_ROW_HEIGHT), root);
        self.write(FIELD_ATTACHED, 1u32);
        self.apply_count();
    }

    unsafe fn detach(&mut self) {
        let detach: extern "thiscall" fn(u32) = std::mem::transmute(SCROLLBAR_DETACH);
        detach(self.bar());
        self.write(FIELD_ATTACHED, 0u32);
    }

    /// The bar only dirties itself when it moves; the rows it scrolls are the window's.
    unsafe fn update(&mut self, window: &GameWindow) {
        if self.poll() {
            window.invalidate();
        }
    }
}
