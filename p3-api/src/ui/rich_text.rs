use std::mem;

/// Draw marked-up text into a rectangle through the UI framework's rich-text pass
/// (`0x00420A10`, thiscall). This is what the letter windows and the tavern's side room
/// use for prose: it lays the string out with word wrap, expands the markup escapes, and
/// draws the resulting lines.
///
/// `layout_object` is an instance of the text-layout class (vtable `0x0066E36C`,
/// constructor `0x004624A0`; **`0x004624D0` is its destructor**, which writes the vtable
/// first like every destructor in this family), which holds the laid-out lines - 28 bytes
/// each, in the vector at `+0x10` with the count at `+0x14`. Windows that draw prose own
/// one, usually through the thin subclass with vtable `0x0066C44C` (constructor
/// `0x004209E0`, destructor `0x00420A00`) that embeds it at `+0`: the tavern keeps its at
/// `window + 0x1608`, and reusing that one is safe while a page other than the side room is
/// on screen, since the object is re-laid-out on every draw.
///
/// The layout pass is `0x00462520(this, string, width, 0)`, so `width` is the wrap limit.
///
/// Beware what `x` anchors: the draw offsets each line by its alignment
/// (`0x00420A97`..`0x00420AB7`) - nothing for a left-aligned line, `(width - line) / 2`
/// for a centred one, and **minus the line's own width** for a right-aligned one. So `x`
/// is the left edge of the text under `\l`, and its right edge under `\r`.
///
/// The markup, from the escape chain at `0x0046264E` onward:
///
/// |Escape|Meaning|
/// |-|-|
/// |`\l` `\r` `\c`|align the line left, right or centre|
/// |`\f` + digit|select a font by index into the global font containers (see [font_escape]); letters begin with `\f1_`|
/// |`\t`|tab|
/// |`\C` `\L` `\B`|inline symbols, from the objects in `0x006CC37C`, `0x006CC384` and `0x006CC380`|
/// |`\d` + `A`..`Z`|an indexed item, looked up in `0x006CC3D4`|
/// |`\h`|substitution delimited by `_`|
///
/// **Fonts are not the ddraw font in effect.** Each run carries a font index, applied at
/// draw time (`0x00420AD6`: index × `0xA8` into [crate::ui::font::FONT_CONTAINER_BASE],
/// then `0x004C1B90`/`0x004BB8F0`). A run without a `\f` uses the layout object's default
/// byte at `+0x4`, which the constructor sets to **0**, `tiepolo_black16` - the heading
/// face - so plain rich text renders heavier than the body font unless it selects
/// [crate::ui::font::NORMAL_FONT_INDEX] itself. The default alignment byte at `+0x5` is 1
/// (left). The draw also forces text mode 1 and its own colour (`0x00420A4E`..`0x00420A5E`).
///
/// # Safety
/// `layout_object` must be a live instance of that class, and `text` must be NUL
/// terminated and in the game's latin1 codepage.
pub unsafe fn draw_rich_text(layout_object: u32, text: &[u8], x: i32, y: i32, width: i32, height: i32, color: u32) {
    // The game's string object is one dword passed by value. Its constructor copies the
    // characters (0x0064F2C1), and the draw destroys the parameter itself (it calls the
    // destructor 0x0064F253), so freeing it here would be a double free.
    let construct_string: extern "thiscall" fn(*mut u32, *const u8) = mem::transmute(0x0064F2C1);
    let draw: extern "thiscall" fn(u32, u32, i32, i32, i32, i32, u32) = mem::transmute(0x00420A10);

    let mut string: u32 = 0;
    construct_string(&mut string, text.as_ptr());
    draw(layout_object, string, x, y, width, height, color);
}

/// The offset of the tavern window's own text-layout object, the one its side room draws
/// letter bodies with (`0x005D7FF7`).
pub const TAVERN_WINDOW_LAYOUT_OFFSET: u32 = 0x1608;

/// The offset of the town hall window's own text-layout object, the one its pages draw
/// prose with (e.g. `0x005E40C2`, in a function that also writes the window's known
/// fields `+0x1930` and `+0x1945` through the same register).
pub const TOWN_HALL_WINDOW_LAYOUT_OFFSET: u32 = 0x18C8;

/// The offset of the church window's own text-layout object (the `0x0066C44C` subclass),
/// constructed by the window's constructor at `0x005C8A2B`.
pub const CHURCH_WINDOW_LAYOUT_OFFSET: u32 = 0x1D08;

/// The markup that switches the rest of a rich-text line to font container `index`
/// (`\f` + one digit). The parser reads the digit with `atoi` (`0x004626AA`) but consumes
/// exactly one character, so the escape must not be followed by another digit: put it at
/// the very start of the string, before the alignment escape.
pub fn font_escape(index: u32) -> String {
    debug_assert!(index < crate::ui::font::FONT_COUNT);
    format!("\\f{index}")
}
