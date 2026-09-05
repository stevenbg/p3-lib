//! Enlarging a building window beyond the 425 x 510 the game gives every one of them.
//!
//! The UI startup sizes every building window to 425 x 510 (`0x00426D98`) and the building
//! backdrop behind them - the interior picture, the whitening veil, the chains and the
//! wooden frame, one object shared by every building - to 451 x 537 (`0x00426A64`). A
//! window's open method centres the window on its size and lays its widgets out from the
//! edges, so a bigger size written before open takes effect on its own. The backdrop does
//! not follow: its picture is a fixed texture blitted one to one, its frame bars are tiled
//! a fixed number of times, and its veil blits read past their 425 x 510 texture - off the
//! end of the buffer once the backdrop is taller than 537, which crashes in `ddraw_dll`.
//!
//! So an enlargement is four calls from the window's own hooks plus the hooks below:
//! [before_open] with the new size, [after_open] and [on_close] to grow and restore the
//! backdrop around its centre, and [install_backdrop_hooks] once from `start()`, which
//! puts [on_backdrop_drawn] after the backdrop's draw and redirects its veil blits.
//!
//! **The backdrop is one object and these hooks are process-wide.** Every mod that enlarges a
//! window installs its own copy of them, so with two such mods the hooks chain: each mod's
//! fill runs only while its own window is open, and the veil redirection is the same code in
//! both, so the result is right either way - but the veil blit of whichever mod hooked last
//! is the one that runs. Once more than one window is enlarged, a single owner of the
//! backdrop hooks (the way `mod-hotkey-registry` owns the keys) would be the cleaner shape.

use std::{
    mem,
    sync::atomic::{AtomicBool, AtomicI32, AtomicPtr, Ordering},
};

use hooklet::windows::x86::{hook_call_rel32, hook_function_pointer, CallRel32Hook, FunctionPointerHook};
use log::error;
use p3_api::{
    data::building_backdrop::BuildingBackdropPtr,
    ui::{
        graphics::{blit, record_frame, select_texture, set_blit_color, stretch_blit, texture_size, OPAQUE_WHITE},
        page_window::PageWindow,
        widget,
    },
};

/// The size the game gives every building window.
pub const VANILLA_WIDTH: i32 = 425;
pub const VANILLA_HEIGHT: i32 = 510;
/// The size the game gives the backdrop.
const BACKDROP_VANILLA_WIDTH: i32 = 451;
const BACKDROP_VANILLA_HEIGHT: i32 = 537;

/// Whether the backdrop is currently enlarged by this mod, and by how much.
static ENLARGED: AtomicBool = AtomicBool::new(false);
static EXTRA_WIDTH: AtomicI32 = AtomicI32::new(0);
static EXTRA_HEIGHT: AtomicI32 = AtomicI32::new(0);

static DRAW_HOOK: AtomicPtr<FunctionPointerHook> = AtomicPtr::new(std::ptr::null_mut());
static VEIL_BLIT_HOOKS: [AtomicPtr<CallRel32Hook>; 2] = [AtomicPtr::new(std::ptr::null_mut()), AtomicPtr::new(std::ptr::null_mut())];

/// Before the game's open method, which reads the size to centre and lay out the window.
pub unsafe fn before_open<W: PageWindow>(window: &W, width: i32, height: i32) {
    widget::set_size(window.address(), width, height);
}

/// After the game's open method: grow the backdrop by the window's excess over the vanilla
/// size, around its centre, and submit the new rectangle for repainting (the game submitted
/// the vanilla one before this). Idempotent - only a backdrop at its vanilla width is
/// touched.
pub unsafe fn after_open(width: i32, height: i32) {
    let extra_width = width - VANILLA_WIDTH;
    let extra_height = height - VANILLA_HEIGHT;
    let backdrop = BuildingBackdropPtr::new();
    let (current_width, current_height) = widget::size(backdrop.address);
    if current_width != BACKDROP_VANILLA_WIDTH {
        return;
    }
    let (x, y) = widget::position(backdrop.address);
    widget::set_size(backdrop.address, current_width + extra_width, current_height + extra_height);
    widget::set_position(backdrop.address, x - extra_width / 2, y - extra_height / 2);
    EXTRA_WIDTH.store(extra_width, Ordering::Relaxed);
    EXTRA_HEIGHT.store(extra_height, Ordering::Relaxed);
    ENLARGED.store(true, Ordering::Relaxed);
    backdrop.clip_stuff();
}

/// On the window's close: the backdrop back to its vanilla size, for the other buildings.
pub unsafe fn on_close() {
    if !ENLARGED.swap(false, Ordering::Relaxed) {
        return;
    }
    let extra_width = EXTRA_WIDTH.load(Ordering::Relaxed);
    let extra_height = EXTRA_HEIGHT.load(Ordering::Relaxed);
    let backdrop = BuildingBackdropPtr::new().address;
    let (width, _) = widget::size(backdrop);
    if width != BACKDROP_VANILLA_WIDTH + extra_width {
        return;
    }
    let (x, y) = widget::position(backdrop);
    widget::set_size(backdrop, BACKDROP_VANILLA_WIDTH, BACKDROP_VANILLA_HEIGHT);
    widget::set_position(backdrop, x + extra_width / 2, y + extra_height / 2);
}

/// Hook the backdrop's draw (vtable `+0x9C`) to run [on_backdrop_drawn] after it, and its two
/// veil blits to [veil_blit]. Once per mod, from `start()`; `Err` carries the step that
/// failed (1 the draw, 2 the blits).
pub unsafe fn install_backdrop_hooks() -> Result<(), u32> {
    match hook_function_pointer(BuildingBackdropPtr::VTABLE_OFFSET + widget::SLOT_DRAW as u32, backdrop_draw_hook as *const () as usize as u32) {
        Ok(hook) => DRAW_HOOK.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
        Err(_) => {
            error!("failed to hook the building backdrop's draw");
            return Err(1);
        }
    }
    for (i, offset) in [BuildingBackdropPtr::VEIL_RAMP_BLIT_CALL_OFFSET, BuildingBackdropPtr::VEIL_FLAT_BLIT_CALL_OFFSET]
        .into_iter()
        .enumerate()
    {
        match hook_call_rel32(offset, veil_blit as *const () as usize as u32) {
            Ok(hook) => VEIL_BLIT_HOOKS[i].store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
            Err(_) => {
                error!("failed to hook the backdrop's veil blit at module+{offset:#x}");
                return Err(2);
            }
        }
    }
    Ok(())
}

/// The backdrop's draw: `thiscall(context, x, y, z)`, `ret 0x10`.
unsafe extern "thiscall" fn backdrop_draw_hook(backdrop: u32, context: u32, x: i32, y: i32, z: u32) {
    let orig: extern "thiscall" fn(u32, u32, i32, i32, u32) = mem::transmute((*DRAW_HOOK.load(Ordering::SeqCst)).old_absolute);
    orig(backdrop, context, x, y, z);
    on_backdrop_drawn();
}

/// After the backdrop's own draw, while enlarged: paint the building picture again, scaled
/// to the enlarged veiled area, over the game's one-to-one copy and its veil; veil it again;
/// draw the whole frame on top.
pub unsafe fn on_backdrop_drawn() {
    if !ENLARGED.load(Ordering::Relaxed) {
        return;
    }
    let backdrop = BuildingBackdropPtr::new();
    let Some(picture) = backdrop.current_picture_handle() else {
        return;
    };
    let (picture_width, picture_height) = texture_size(picture);
    let (x, y) = widget::position(backdrop.address);
    let (width, height) = widget::size(backdrop.address);
    let inset = BuildingBackdropPtr::PICTURE_INSET;
    // The veiled area: from the inset to the veil's right and bottom margins.
    let area_width = width - BuildingBackdropPtr::VEIL_WIDTH_MARGIN;
    let area_height = height - BuildingBackdropPtr::VEIL_HEIGHT_MARGIN;
    if picture_width <= 0 || picture_height <= 0 || area_width <= picture_width && area_height <= picture_height {
        return;
    }

    select_texture(picture);
    set_blit_color(OPAQUE_WHITE);
    stretch_blit(0, 0, x + inset, y + inset, picture_width, picture_height, area_width, area_height);
    veil(&backdrop, y, height, x + inset, area_width, y + inset, y + inset + area_height);
    set_blit_color(OPAQUE_WHITE);

    draw_frame(&backdrop, x, y, width, height);
}

/// The veil as the game draws it (`0x00465DC0`..`0x00465EAD`), restricted to the columns
/// `left..left + width` and the rows `top..bottom`: the fade-in rows above the gradient y,
/// then the flat sheet from the gradient y down.
unsafe fn veil(backdrop: &BuildingBackdropPtr, y: i32, height: i32, left: i32, width: i32, top: i32, bottom: i32) {
    let gradient = backdrop.gradient_y() as i32;
    let texture = backdrop.veil_texture_handle();
    let (texture_width, texture_height) = texture_size(texture);
    select_texture(texture);
    if gradient != 0 {
        for row in 0..BuildingBackdropPtr::VEIL_RAMP_ROWS {
            let row_y = y + gradient + row - BuildingBackdropPtr::VEIL_RAMP_OFFSET;
            let source_y = gradient + row - BuildingBackdropPtr::VEIL_RAMP_ROWS;
            if row_y <= y || source_y < 0 || row_y < top || row_y >= bottom {
                continue;
            }
            set_blit_color(((row as u32) << 24) | 0x00FF_FFFF);
            sheet(texture_width, texture_height, 0, source_y, left, row_y, width, 1);
        }
    }
    let flat_top = (y + BuildingBackdropPtr::PICTURE_INSET + gradient).max(top);
    let flat_bottom = (y + BuildingBackdropPtr::PICTURE_INSET + height - BuildingBackdropPtr::VEIL_HEIGHT_MARGIN).min(bottom);
    if flat_bottom > flat_top {
        set_blit_color(BuildingBackdropPtr::VEIL_COLOR);
        sheet(
            texture_width,
            texture_height,
            0,
            flat_top - (y + BuildingBackdropPtr::PICTURE_INSET),
            left,
            flat_top,
            width,
            flat_bottom - flat_top,
        );
    }
}

/// A piece of the veil over a rectangle that may exceed the texture: the texture from the
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

/// The game's own veil blits, redirected here: the same rectangle, but scaled over the
/// texture instead of read past it - the one-row ramp blits sideways, the flat sheet both
/// ways, so the paper pattern is continuous across both. Harmless at the vanilla size, where
/// nothing exceeds the texture.
unsafe extern "C" fn veil_blit(source_x: i32, source_y: i32, x: i32, y: i32, width: i32, height: i32) {
    let (texture_width, texture_height) = texture_size(BuildingBackdropPtr::new().veil_texture_handle());
    sheet(texture_width, texture_height, source_x, source_y, x, y, width, height);
}

/// Draw the whole frame again, on top of the scaled picture: the game's frame went under it,
/// and its bars are tiled a fixed number of times anyway, which covers the vanilla size
/// only. Bars run from the inset to the far corner, the last tile clipped; the corners go on
/// last.
unsafe fn draw_frame(backdrop: &BuildingBackdropPtr, x: i32, y: i32, width: i32, height: i32) {
    let record = backdrop.frame_record();
    if record == 0 {
        return;
    }
    let Some((handle, _, _, corner_width, corner_height)) = record_frame(record, BuildingBackdropPtr::FRAME_CORNER_TOP_RIGHT) else {
        return;
    };
    let corner_x = x + width - corner_width;
    let corner_y = y + height - corner_height;
    select_texture(handle);
    set_blit_color(OPAQUE_WHITE);

    for (frame, along_bottom) in [(BuildingBackdropPtr::FRAME_TOP_BAR, false), (BuildingBackdropPtr::FRAME_BOTTOM_BAR, true)] {
        let Some((_, source_x, source_y, piece_width, piece_height)) = record_frame(record, frame) else {
            continue;
        };
        if piece_width <= 0 {
            continue;
        }
        let bar_y = if along_bottom { y + height - piece_height } else { y };
        let mut tile_x = x + BuildingBackdropPtr::PICTURE_INSET;
        while tile_x < corner_x {
            let visible = piece_width.min(corner_x - tile_x);
            blit(source_x, source_y, tile_x, bar_y, visible, piece_height);
            tile_x += piece_width;
        }
    }
    for (frame, along_right) in [(BuildingBackdropPtr::FRAME_LEFT_BAR, false), (BuildingBackdropPtr::FRAME_RIGHT_BAR, true)] {
        let Some((_, source_x, source_y, piece_width, piece_height)) = record_frame(record, frame) else {
            continue;
        };
        if piece_height <= 0 {
            continue;
        }
        let bar_x = if along_right { x + width - piece_width } else { x };
        let mut tile_y = y + BuildingBackdropPtr::PICTURE_INSET;
        while tile_y < corner_y {
            let visible = piece_height.min(corner_y - tile_y);
            blit(source_x, source_y, bar_x, tile_y, piece_width, visible);
            tile_y += piece_height;
        }
    }
    for (frame, along_right, along_bottom) in [
        (BuildingBackdropPtr::FRAME_CORNER_TOP_LEFT, false, false),
        (BuildingBackdropPtr::FRAME_CORNER_TOP_RIGHT, true, false),
        (BuildingBackdropPtr::FRAME_CORNER_BOTTOM_RIGHT, true, true),
        (BuildingBackdropPtr::FRAME_CORNER_BOTTOM_LEFT, false, true),
    ] {
        let Some((_, source_x, source_y, piece_width, piece_height)) = record_frame(record, frame) else {
            continue;
        };
        let corner_x = if along_right { x + width - piece_width } else { x };
        let corner_y = if along_bottom { y + height - piece_height } else { y };
        blit(source_x, source_y, corner_x, corner_y, piece_width, piece_height);
    }
}
