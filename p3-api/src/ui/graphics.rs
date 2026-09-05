use std::mem;

/// The resource manager the game fetches graphics from.
const GRAPHICS_MANAGER: u32 = 0x006DA820;

/// Graphic ids of the icons the building pages draw beside their numbers. They are not
/// constants in the executable: the windows read them by name from `scripts/parchment.ini`
/// and `scripts/BuildingParchment.ini` inside `p2arch0_eng.cpr`, each building having its
/// own section - the shipyard's `[Werftparchment]`, the tavern's `[Kneipeparchment]`. The
/// values below are vanilla 1.1; a mod that edits those files would change them, in which
/// case they would have to be read through the game's own ini lookup (`0x004BE0B0`).
///
/// `scripts/textures.ini` maps each id to its image: `CrewID` is `[TEX20017]`,
/// `images/frames_listen/crew0002.tga` at offset 0,0 sized 26x18.
pub const GRAPHIC_ID_WARES: u32 = 16046;
pub const GRAPHIC_ID_MONEY: u32 = 16047;
pub const GRAPHIC_ID_CONVOY: u32 = 16043;
pub const GRAPHIC_ID_CAPTAIN: u32 = 16053;
/// The pirate figure (`[TEX16003]`, `images/frames_listen/pirat0001.tga`, 26x18). Not one
/// of the parchment keys - taken straight from `scripts/textures.ini`.
pub const GRAPHIC_ID_PIRATE: u32 = 16003;
pub const GRAPHIC_ID_ARMAMENT_SMALL: u32 = 16054;
pub const GRAPHIC_ID_ARMAMENT_LARGE: u32 = 16056;
pub const GRAPHIC_ID_HEART: u32 = 20013;
pub const GRAPHIC_ID_KNOTS: u32 = 20016;
pub const GRAPHIC_ID_CREW: u32 = 20017;
pub const GRAPHIC_ID_TIME: u32 = 32001;
/// The side menu's three skill-bonus icons in one sheet (`[TEX20011]`,
/// `images/sidemenu/bonus.tga`): three 16x16 frames at (0,0), (16,0) and (0,16).
pub const GRAPHIC_ID_BONUS: u32 = 20011;
/// Frames of `GRAPHIC_ID_BONUS`, in the order the sheet stores them.
pub const BONUS_FRAME_0: u32 = 0;
pub const BONUS_FRAME_1: u32 = 1;
pub const BONUS_FRAME_2: u32 = 2;

/// Draw a game graphic by id, with its top left corner at `x`, `y`. Returns whether the
/// graphic was there to draw.
///
/// This is the sequence the windows use for their own icons - the tavern at `0x005CDD07`,
/// the shipyard at `0x005F4ECE`, `render_window_title` at `0x00420C8C`:
///
/// 1. `0x004B3DD0` (thiscall on the manager) fetches the graphic record for an id. Its
///    `+0x4` is the handle the renderer works with, and it only counts as usable while
///    `+0x14` is positive and the handle is non-null - both guards the game itself applies.
/// 2. `0x004BBB20(handle, &size)` measures it into two dwords, width then height.
/// 3. `0x004BB9C0(handle)` selects it, the way `ddraw_set_font` selects a font.
/// 4. `0x004BB870(0xFFFFFFFF)` sets the constant colour, because the blit modulates the
///    image by it - with a text colour still set the icon comes out as a silhouette
///    (`render_window_title` does this at `0x00420CDB`).
/// 5. `0x004BB330(src_x, src_y, x, y, width, height)` blits it. The source offset and size
///    are what `scripts/textures.ini` records per graphic as `OffsetNSize0`.
///
/// The colour is left as it was set here, so a caller that draws text afterwards has to
/// set its own colour again.
///
/// # Safety
/// Only safe while the renderer is drawing - inside a window's draw method.
pub unsafe fn draw_graphic(id: u32, x: i32, y: i32) -> bool {
    draw_graphic_frame(id, 0, x, y)
}

/// Draw one frame of a graphic. A graphic sheet holds several pictures - the record's
/// `+0x14` is the frame count and its `+0xC` points at an array of four-dword rects, one
/// per frame, holding source x, source y, width and height. Those are the `OffsetNSize0`,
/// `OffsetNSize1`, ... entries of `scripts/textures.ini`, so blitting a frame is blitting
/// its rect out of the shared texture.
pub unsafe fn draw_graphic_frame(id: u32, frame: u32, x: i32, y: i32) -> bool {
    let Some((handle, source_x, source_y, width, height)) = graphic_frame(id, frame) else {
        return false;
    };

    select_texture(handle);
    set_blit_color(OPAQUE_WHITE);
    blit(source_x, source_y, x, y, width, height);
    true
}

/// The constant colour that leaves a blit unmodulated.
pub const OPAQUE_WHITE: u32 = 0xFFFF_FFFF;

/// Select a texture by its renderer handle (`0x004BB9C0`), the record's `+0x4`.
pub unsafe fn select_texture(handle: u32) {
    let select: extern "cdecl" fn(u32) = mem::transmute(0x004BB9C0);
    select(handle);
}

/// The constant colour the next blits are modulated by (`0x004BB870`); the same setter the
/// text drawing uses, so text drawn afterwards has to set its colour again.
pub unsafe fn set_blit_color(color: u32) {
    let set_color: extern "cdecl" fn(u32) = mem::transmute(0x004BB870);
    set_color(color);
}

/// Copy `width x height` pixels from `source_x, source_y` of the selected texture to
/// `x, y` (`0x004BB330`), modulated by the constant colour. The copy is one to one; a
/// source rectangle larger than the texture is not stretched.
pub unsafe fn blit(source_x: i32, source_y: i32, x: i32, y: i32, width: i32, height: i32) {
    let blit: extern "cdecl" fn(i32, i32, i32, i32, i32, i32) = mem::transmute(0x004BB330);
    blit(source_x, source_y, x, y, width, height);
}

/// Blit `source_width x source_height` pixels from `source_x, source_y` of the selected
/// texture into the `width x height` rectangle at `x, y`, scaled (`sgl_StretchBitmapRect`,
/// thunk `0x004BBA80`, modulated by the constant colour like [blit]). The argument order is
/// the game's own: the minimap builder at `0x004B1463` copies a whole measured texture
/// into a memory texture of its own size with `(0, 0, 0, 0, tex_w, tex_h, dst_w, dst_h)`,
/// and `0x004B27B5` stretches from `(0, 0)` to a destination rectangle.
pub unsafe fn stretch_blit(source_x: i32, source_y: i32, x: i32, y: i32, source_width: i32, source_height: i32, width: i32, height: i32) {
    let stretch: extern "cdecl" fn(i32, i32, i32, i32, i32, i32, i32, i32) = mem::transmute(0x004BBA80);
    stretch(source_x, source_y, x, y, source_width, source_height, width, height);
}

/// Width and height of a texture by handle (`0x004BBB20`).
pub unsafe fn texture_size(handle: u32) -> (i32, i32) {
    let mut size = [0i32; 2];
    let measure: extern "cdecl" fn(u32, *mut i32) = mem::transmute(0x004BBB20);
    measure(handle, size.as_mut_ptr());
    (size[0], size[1])
}

/// The size of a graphic's frame, for laying text out around it - the icons are not one
/// size: the bonus sheet's frames are 16x16, a captain 18x18, the coin, crew figure and
/// pirate 26x18.
pub unsafe fn graphic_frame_size(id: u32, frame: u32) -> Option<(i32, i32)> {
    graphic_frame(id, frame).map(|(_, _, _, width, height)| (width, height))
}

/// The renderer handle and the frame's source rect, with the guards the game applies.
unsafe fn graphic_frame(id: u32, frame: u32) -> Option<(u32, i32, i32, i32, i32)> {
    let fetch: extern "thiscall" fn(u32, u32) -> u32 = mem::transmute(0x004B3DD0);
    let graphic = fetch(GRAPHICS_MANAGER, id);
    if graphic == 0 {
        return None;
    }
    record_frame(graphic, frame)
}

/// [graphic_frame] for a graphic record already in hand - the records the windows cache in
/// their own fields. Returns `(handle, source_x, source_y, width, height)`.
pub unsafe fn record_frame(graphic: u32, frame: u32) -> Option<(u32, i32, i32, i32, i32)> {
    let frames: i32 = *((graphic + 0x14) as *const i32);
    let handle: u32 = *((graphic + 0x4) as *const u32);
    let rects: u32 = *((graphic + 0xc) as *const u32);
    if frames <= 0 || frame >= frames as u32 || handle == 0 || !(0x0001_0000..0x7fff_0000).contains(&rects) {
        return None;
    }
    let rect = rects + frame * 0x10;
    Some((
        handle,
        *(rect as *const i32),
        *((rect + 4) as *const i32),
        *((rect + 8) as *const i32),
        *((rect + 0xc) as *const i32),
    ))
}
