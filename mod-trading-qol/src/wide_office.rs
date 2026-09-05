//! A bigger trading office window, so the administrator page can hold one more column and
//! more rows.
//!
//! The UI startup sizes every building window to 425 x 510 (`0x00426D98`) and the building
//! backdrop behind them - the interior picture, the whitening veil, the chains and the
//! wooden frame, one object shared by every building - to 451 x 537 (`0x00426A64`). The
//! window's open method centres the window on its size and lays every right-anchored row
//! widget out from the right edge, so enlarging the window before open opens the extra room
//! in the middle of the rows. The backdrop is enlarged by the same amounts around its centre
//! while the office is open, and given back to the other buildings on close.
//!
//! The backdrop's own draw does not scale: the picture is a fixed texture blitted one to
//! one, and the frame bars are tiled a fixed number of times. A hook after its draw fills
//! what that leaves open - the picture's outer rows and columns repeated into the strips,
//! veiled as the game veils the rest, and the frame bars continued to their corners.

use std::sync::atomic::{AtomicBool, Ordering};

use p3_api::{
    data::class48::Class48Ptr,
    ui::{
        graphics::{blit, record_frame, select_texture, set_blit_color, stretch_blit, texture_size, OPAQUE_WHITE},
        ui_trading_office_window::UITradingOfficeWindowPtr,
        widget,
    },
};

/// Whether the backdrop is currently enlarged for the office, so its draw hook knows to fill
/// the strips the building picture and the frame leave open.
static WIDENED: AtomicBool = AtomicBool::new(false);

/// The size the game gives the window.
const VANILLA_WIDTH: i32 = 425;
const VANILLA_HEIGHT: i32 = 510;
/// The size the game gives the backdrop.
const BACKDROP_VANILLA_WIDTH: i32 = 451;
const BACKDROP_VANILLA_HEIGHT: i32 = 537;
/// The size the office window opens at.
pub const WIDTH: i32 = 490;
pub const HEIGHT: i32 = 560;
const EXTRA_WIDTH: i32 = WIDTH - VANILLA_WIDTH;
const EXTRA_HEIGHT: i32 = HEIGHT - VANILLA_HEIGHT;

/// Before the game's open, which reads the size to centre and lay out the window.
pub unsafe fn before_open(window: &UITradingOfficeWindowPtr) {
    widget::set_size(window.address, WIDTH, HEIGHT);
}

/// After the game's open: enlarge the backdrop around its centre. Idempotent - only a
/// backdrop at its vanilla size is touched.
pub unsafe fn after_open() {
    let backdrop = Class48Ptr::new();
    let (width, height) = widget::size(backdrop.address);
    if width != BACKDROP_VANILLA_WIDTH {
        return;
    }
    let (x, y) = widget::position(backdrop.address);
    widget::set_size(backdrop.address, width + EXTRA_WIDTH, height + EXTRA_HEIGHT);
    widget::set_position(backdrop.address, x - EXTRA_WIDTH / 2, y - EXTRA_HEIGHT / 2);
    WIDENED.store(true, Ordering::Relaxed);
    // The game submitted the vanilla rectangle for repainting before this; submit the new one
    // so the enlarged sides are drawn on the first frame rather than on the next mouse move.
    backdrop.clip_stuff();
}

/// On close: the backdrop back to its vanilla size, for the other buildings.
pub unsafe fn on_close() {
    WIDENED.store(false, Ordering::Relaxed);
    let backdrop = Class48Ptr::new().address;
    let (width, height) = widget::size(backdrop);
    if width != BACKDROP_VANILLA_WIDTH + EXTRA_WIDTH {
        return;
    }
    let _ = height;
    let (x, y) = widget::position(backdrop);
    widget::set_size(backdrop, BACKDROP_VANILLA_WIDTH, BACKDROP_VANILLA_HEIGHT);
    widget::set_position(backdrop, x + EXTRA_WIDTH / 2, y + EXTRA_HEIGHT / 2);
}

/// After the backdrop's own draw: paint the building picture again, scaled to the enlarged
/// veiled area, over the game's one-to-one copy and its veil; veil it again; finish the frame.
pub unsafe fn on_backdrop_drawn() {
    if !WIDENED.load(Ordering::Relaxed) {
        return;
    }
    let backdrop = Class48Ptr::new();
    let Some(picture) = backdrop.current_picture_handle() else {
        return;
    };
    let (picture_width, picture_height) = texture_size(picture);
    let (x, y) = widget::position(backdrop.address);
    let (width, height) = widget::size(backdrop.address);
    let inset = Class48Ptr::PICTURE_INSET;
    // The veiled area: from the inset to the veil's right and bottom margins.
    let area_width = width - Class48Ptr::VEIL_WIDTH_MARGIN;
    let area_height = height - Class48Ptr::VEIL_HEIGHT_MARGIN;
    if picture_width <= 0 || picture_height <= 0 || area_width <= picture_width && area_height <= picture_height {
        return;
    }

    select_texture(picture);
    set_blit_color(OPAQUE_WHITE);
    stretch_blit(0, 0, x + inset, y + inset, picture_width, picture_height, area_width, area_height);
    veil(&backdrop, y, height, x + inset, area_width, y + inset, y + inset + area_height);
    set_blit_color(OPAQUE_WHITE);

    complete_frame(&backdrop, x, y, width, height);
}

/// The veil as the game draws it (`0x00465DC0`..`0x00465EAD`), restricted to the columns
/// `left..left + width` and the rows `top..bottom`: the fade-in rows above the gradient y,
/// then the flat sheet from the gradient y down.
unsafe fn veil(backdrop: &Class48Ptr, y: i32, height: i32, left: i32, width: i32, top: i32, bottom: i32) {
    let gradient = backdrop.gradient_y() as i32;
    let texture = backdrop.veil_texture_handle();
    let (texture_width, texture_height) = texture_size(texture);
    select_texture(texture);
    if gradient != 0 {
        for row in 0..Class48Ptr::VEIL_RAMP_ROWS {
            let row_y = y + gradient + row - Class48Ptr::VEIL_RAMP_OFFSET;
            let source_y = gradient + row - Class48Ptr::VEIL_RAMP_ROWS;
            if row_y <= y || source_y < 0 || row_y < top || row_y >= bottom {
                continue;
            }
            set_blit_color(((row as u32) << 24) | 0x00FF_FFFF);
            sheet(texture_width, texture_height, 0, source_y, left, row_y, width, 1);
        }
    }
    let flat_top = (y + Class48Ptr::PICTURE_INSET + gradient).max(top);
    let flat_bottom = (y + Class48Ptr::PICTURE_INSET + height - Class48Ptr::VEIL_HEIGHT_MARGIN).min(bottom);
    if flat_bottom > flat_top {
        set_blit_color(Class48Ptr::VEIL_COLOR);
        sheet(texture_width, texture_height, 0, flat_top - (y + Class48Ptr::PICTURE_INSET), left, flat_top, width, flat_bottom - flat_top);
    }
}

/// The veil's flat sheet over a rectangle that may exceed the texture: the texture from the
/// source offset down, scaled to the rectangle, so the paper shows no seam.
unsafe fn sheet(texture_width: i32, texture_height: i32, source_x: i32, source_y: i32, x: i32, y: i32, width: i32, height: i32) {
    let source_width = texture_width - source_x;
    let source_height = texture_height - source_y;
    if source_width <= 0 || source_height <= 0 {
        return;
    }
    if width <= source_width && height <= source_height {
        blit(source_x, source_y, x, y, width, height);
    } else {
        stretch_blit(source_x, source_y, x, y, source_width, source_height, width, height);
    }
}

/// The game's own veil blits, redirected here while the backdrop is enlarged: the same
/// rectangle, but scaled over the texture instead of read past it - the one-row ramp blits
/// sideways, the flat sheet both ways, so the paper pattern is continuous across both.
pub unsafe extern "C" fn veil_blit(source_x: i32, source_y: i32, x: i32, y: i32, width: i32, height: i32) {
    let (texture_width, texture_height) = texture_size(Class48Ptr::new().veil_texture_handle());
    sheet(texture_width, texture_height, source_x, source_y, x, y, width, height);
}

/// Draw the whole frame again, on top of the scaled picture: the game's frame went under it,
/// and its bars are tiled a fixed number of times anyway, which covers the vanilla size
/// only. Bars run from the inset to the far corner, the last tile clipped; the corners go on
/// last.
unsafe fn complete_frame(backdrop: &Class48Ptr, x: i32, y: i32, width: i32, height: i32) {
    let record = backdrop.frame_record();
    if record == 0 {
        return;
    }
    let Some((handle, _, _, corner_width, corner_height)) = record_frame(record, Class48Ptr::FRAME_CORNER_TOP_RIGHT) else {
        return;
    };
    let corner_x = x + width - corner_width;
    let corner_y = y + height - corner_height;
    select_texture(handle);
    set_blit_color(OPAQUE_WHITE);

    for (frame, along_bottom) in [(Class48Ptr::FRAME_TOP_BAR, false), (Class48Ptr::FRAME_BOTTOM_BAR, true)] {
        let Some((_, source_x, source_y, piece_width, piece_height)) = record_frame(record, frame) else {
            continue;
        };
        if piece_width <= 0 {
            continue;
        }
        let bar_y = if along_bottom { y + height - piece_height } else { y };
        let mut tile_x = x + Class48Ptr::PICTURE_INSET;
        while tile_x < corner_x {
            let visible = piece_width.min(corner_x - tile_x);
            blit(source_x, source_y, tile_x, bar_y, visible, piece_height);
            tile_x += piece_width;
        }
    }
    for (frame, along_right) in [(Class48Ptr::FRAME_LEFT_BAR, false), (Class48Ptr::FRAME_RIGHT_BAR, true)] {
        let Some((_, source_x, source_y, piece_width, piece_height)) = record_frame(record, frame) else {
            continue;
        };
        if piece_height <= 0 {
            continue;
        }
        let bar_x = if along_right { x + width - piece_width } else { x };
        let mut tile_y = y + Class48Ptr::PICTURE_INSET;
        while tile_y < corner_y {
            let visible = piece_height.min(corner_y - tile_y);
            blit(source_x, source_y, bar_x, tile_y, piece_width, visible);
            tile_y += piece_height;
        }
    }
    for (frame, along_right, along_bottom) in [
        (Class48Ptr::FRAME_CORNER_TOP_LEFT, false, false),
        (Class48Ptr::FRAME_CORNER_TOP_RIGHT, true, false),
        (Class48Ptr::FRAME_CORNER_BOTTOM_RIGHT, true, true),
        (Class48Ptr::FRAME_CORNER_BOTTOM_LEFT, false, true),
    ] {
        let Some((_, source_x, source_y, piece_width, piece_height)) = record_frame(record, frame) else {
            continue;
        };
        let corner_x = if along_right { x + width - piece_width } else { x };
        let corner_y = if along_bottom { y + height - piece_height } else { y };
        blit(source_x, source_y, corner_x, corner_y, piece_width, piece_height);
    }
}
