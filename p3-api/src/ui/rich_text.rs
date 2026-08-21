use std::mem;

/// Draw marked-up text into a rectangle through the UI framework's rich-text pass
/// (`0x00420A10`, thiscall). This is what the letter windows and the tavern's side room
/// use for prose: it lays the string out with word wrap, expands the markup escapes, and
/// draws the resulting lines.
///
/// `layout_object` is an instance of the text-layout class (vtable `0x0066E36C`,
/// constructor `0x004624D0`), which holds the laid-out lines - 28 bytes each, in the
/// vector at `+0x10` with the count at `+0x14`. Windows that draw prose own one: the
/// tavern keeps its at `window + 0x1608`, and reusing that one is safe while a page other
/// than the side room is on screen, since the object is re-laid-out on every draw.
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
/// |`\f`|select a font (letters begin with `\f1_`)|
/// |`\t`|tab|
/// |`\C` `\L` `\B`|inline symbols, from the objects in `0x006CC37C`, `0x006CC384` and `0x006CC380`|
/// |`\d` + `A`..`Z`|an indexed item, looked up in `0x006CC3D4`|
/// |`\h`|substitution delimited by `_`|
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
