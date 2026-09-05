use std::{ffi::c_void, mem};

use crate::data::p3_ptr::P3Pointer;

/// The alignment `ui_render_text_at` applies to the x it is given: `AlignRight` draws the
/// text ending at x, `AlignLeft` starting there.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextMode {
    AlignCenter = 0,
    AlignLeft = 1,
    AlignRight = 2,
}

#[derive(Debug, Clone, Copy)]
pub struct DdrawFontPtr {
    pub address: u32,
}

impl DdrawFontPtr {
    pub fn new(address: u32) -> Self {
        Self { address }
    }

    pub fn next_office_id(&self) -> u16 {
        unsafe { self.get(0x2ca) }
    }
}

impl P3Pointer for DdrawFontPtr {
    fn get_address(&self) -> u32 {
        self.address
    }
}

#[derive(Debug)]
pub struct DdrawFontContainerPtr {
    pub address: u32,
}

impl DdrawFontContainerPtr {
    pub fn new(address: u32) -> Self {
        Self { address }
    }

    pub fn get_font(&self) -> DdrawFontPtr {
        unsafe { DdrawFontPtr::new(self.get(0x98)) }
    }
}

impl P3Pointer for DdrawFontContainerPtr {
    fn get_address(&self) -> u32 {
        self.address
    }
}

pub fn ddraw_set_text_mode(mode: TextMode) {
    let function: extern "cdecl" fn(mode: u32) = unsafe { mem::transmute(0x004BBA10) };
    function(mode as u32)
}

pub fn ddraw_set_font(font: DdrawFontPtr) {
    let function: extern "cdecl" fn(font: *const c_void) = unsafe { mem::transmute(0x004BB8F0) };
    function(font.address as _)
}

/// The game's global fonts, an array of containers of stride `0xA8` based at
/// [FONT_CONTAINER_BASE]. There are **six**, and they are `scripts/fonts.ini`'s `Font0`
/// through `Font5` in order:
///
/// |Index|Address|`fonts.ini`|Face|Params (w h spacing)|
/// |-|-|-|-|-|
/// |0|`0x006DCD28`|`Font0`|`tiepolo_black16.aim`|15 16 0|
/// |1|`0x006DCDD0`|`Font1`|`tiepolo_bold16.aim`|15 16 0|
/// |2|`0x006DCE78`|`Font2`|`tiepolo_black20.aim`|19 20 0|
/// |3|`0x006DCF20`|`Font3`|`elgreco24.aim` (kerning)|26 24 0|
/// |4|`0x006DCFC8`|`Font4`|`tiepolo_bold24.aim`|25 24 0|
/// |5|`0x006DD070`|`Font5`|`elgreco72.aim`|80 75 0|
///
/// All six are referenced from code (458, 290, 90, 60, 30 and 13 times respectively);
/// index 6 is not a font container. The pairing of container index to `fonts.ini` index
/// follows from the counts and the ordering rather than from a decoded loader.
///
/// Note **0 and 1 are the same size** - Tiepolo Black against Tiepolo Bold - so index 0
/// is *heavier* than index 1, not larger.
pub const FONT_CONTAINER_BASE: u32 = 0x006D_CD28;
pub const FONT_CONTAINER_STRIDE: u32 = 0xA8;
pub const FONT_COUNT: u32 = 6;
/// Index of `Font0`, `tiepolo_black16` - the heading face ([get_header_font]).
pub const HEADER_FONT_INDEX: u32 = 0;
/// Index of `Font1`, `tiepolo_bold16` - the body face ([get_normal_font]).
pub const NORMAL_FONT_INDEX: u32 = 1;

/// One of the six global fonts by index; falls back to index 1 when out of range.
pub fn get_font(index: u32) -> DdrawFontPtr {
    let index = if index < FONT_COUNT { index } else { 1 };
    DdrawFontContainerPtr::new(FONT_CONTAINER_BASE + index * FONT_CONTAINER_STRIDE).get_font()
}

/// `Font1`, `tiepolo_bold16` - the body text of the parchment pages and panels.
pub fn get_normal_font() -> DdrawFontPtr {
    get_font(NORMAL_FONT_INDEX)
}

/// `Font0`, `tiepolo_black16` - the same size as [get_normal_font] in the heavier Black
/// weight, which is what the game uses for headings.
pub fn get_header_font() -> DdrawFontPtr {
    get_font(HEADER_FONT_INDEX)
}

/// `Font2`, `tiepolo_black20` - Black weight at 20px, the heaviest face that is still
/// close to body size.
pub fn get_black20_font() -> DdrawFontPtr {
    get_font(2)
}

/// `Font3`, `elgreco24` - a different typeface entirely, kerned, used for the town names
/// drawn over the scrollmap.
pub fn get_scrollmap_town_name_font() -> DdrawFontPtr {
    get_font(3)
}

/// `Font4`, `tiepolo_bold24`.
pub fn get_bold24_font() -> DdrawFontPtr {
    get_font(4)
}

/// `Font5`, `elgreco72` - the title face, 75px tall.
pub fn get_title_font() -> DdrawFontPtr {
    get_font(5)
}
