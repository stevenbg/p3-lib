//! The first stop's load requirement, drawn on the ship panel's barrel views.
//!
//! A route's first stop is where the ship fills up, so the sum of its inbound orders is
//! the number worth comparing against the ship's capacity - the figure the panel already
//! prints beside the barrel. This draws that sum next to it, on the two views that show
//! the barrel: Goods and Auto trade (they share the layout).
//!
//! Nothing is drawn for the other views, so nothing has to be erased when the view
//! changes: the panel repaints itself every frame as part of the scrollmap, and this hook
//! simply adds nothing on the frames where the test fails.
//!
//! **The panel is drawn twice per frame and only one of the passes may be drawn into.**
//! One has the origin arguments at `0,0`, so the panel's own `+0x14`/`+0x18` are already
//! screen coordinates; the other passes the negative of them, rendering the panel at
//! `(0,0)` in a local space. `ddraw_fill_solid_rect` and `ui_render_text_at` take screen
//! coordinates and follow whichever pass is running, so drawing in the local one put this
//! number over the minimap - where nothing repaints it, so it stayed until something else
//! redrew that corner. Hence the origin test in [draw_load_requirement], and see
//! [p3_api::ui::widget] for the general rule.

use std::sync::atomic::{AtomicPtr, AtomicU64, Ordering};

use hooklet::windows::x86::{hook_function_pointer, FunctionPointerHook};
use log::info;
use p3_api::{
    data::{ddraw_fill_solid_rect, ddraw_set_constant_color, ui_render_text_at},
    ui::{font, font::TextMode, ui_ship_panel::UIShipPanelPtr, widget},
};

/// The panel's draw method, vtable slot `+0x9C` (`0x0048B060`). `thiscall` with four
/// stack arguments (`ret 0x10`), all passed straight through.
const DRAW_POINTER_OFFSET: u32 = UIShipPanelPtr::VTABLE_OFFSET + 0x9C;
static DRAW_HOOK: AtomicPtr<FunctionPointerHook> = AtomicPtr::new(std::ptr::null_mut());

/// Where the number goes: right-aligned, just left of the capacity figure the game
/// prints at the panel's top right, so it sits over the barrel. Panel-relative, and the
/// panel measures **260 x 247**.
const LOAD_X: i32 = 217;
const LOAD_Y: i32 = 32;
/// What was last logged, so a per-frame hook reports only when something changes.
static LAST_LOGGED: AtomicU64 = AtomicU64::new(u64::MAX);
/// Black, like the parchment pages' text, rather than the white the panel uses for the
/// figures beside the barrel.
const COLOR: u32 = 0xff00_0000;
/// A parchment-toned plate behind the glyphs, so black text stays readable on the
/// barrel's artwork. `ddraw_fill_solid_rect` takes no colour of its own - it uses
/// whatever `ddraw_set_constant_color` last set.
///
/// The high byte is alpha. Whether the fill honours it is untested - every call the game
/// makes itself passes `0xFF` - so if this reads as opaque, the renderer ignores it and
/// the plate has to be a flat tone chosen to suit the barrel instead.
const BACKGROUND: u32 = 0x80e8_dcc8;
/// Digit advance in `tiepolo_bold16`, from the one-glyph nudge that moved the number 8px,
/// and the line box around it. The game exposes no text-measure call, so the plate is
/// sized from these rather than from the string.
const CHAR_WIDTH: i32 = 8;
const LINE_HEIGHT: i32 = 13;
const PAD_X: i32 = 3;
const PAD_Y: i32 = 1;

pub(crate) unsafe fn install() -> Result<(), &'static str> {
    match hook_function_pointer(DRAW_POINTER_OFFSET, draw_hook as usize as u32) {
        Ok(hook) => {
            DRAW_HOOK.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst);
            Ok(())
        }
        Err(_) => Err("the ship panel's draw method"),
    }
}

/// Draw after the panel, so the number sits on top of its art rather than under it.
///
/// **The hooked slot belongs to the class, not to one object**, so this runs for every
/// widget built from that vtable, and the town view has more than one. The origin arguments
/// describe whichever object is drawing, so they may only be combined with that same
/// object's fields - mixing them with the scrollmap panel's coordinates is what put the
/// number on the minimap, once per extra instance.
#[no_mangle]
unsafe extern "thiscall" fn draw_hook(panel_address: u32, a0: u32, a1: u32, a2: u32, a3: u32) {
    let orig: extern "thiscall" fn(u32, u32, u32, u32, u32) = std::mem::transmute((*DRAW_HOOK.load(Ordering::SeqCst)).old_absolute);
    orig(panel_address, a0, a1, a2, a3);
    // The panel's own origin is `+0x14`/`+0x18` **plus** the second and third stack
    // arguments - the draw method adds them at 0x0048B07D/0x0048B07F before using the
    // result as the screen position, so `+0x14` alone is relative to the parent and
    // lands the text somewhere else entirely.
    draw_load_requirement(panel_address, a1 as i32, a2 as i32);
}

unsafe fn draw_load_requirement(panel_address: u32, origin_x: i32, origin_y: i32) {
    // Only the scrollmap's own ship panel, the object the static names; any other widget
    // sharing the vtable draws its own thing and is none of our business.
    let panel = UIShipPanelPtr::new();
    if panel.address == 0 || panel_address != panel.address {
        return;
    }
    if !widget::is_visible(panel.address) || !panel.is_barrel_view() {
        return;
    }
    // Only the pass that draws in screen space. The panel is also drawn into its own local
    // space, with the origin arguments set to the negative of its `+0x14`/`+0x18` so that it
    // renders at (0,0); our draw calls take screen coordinates and would follow that pass
    // into whatever surface it targets, which is how the number reached the minimap.
    if origin_x != 0 || origin_y != 0 {
        return;
    }
    let Some(ship_index) = panel.get_selected_ship_index() else { return };
    let Some(ship) = panel.get_selected_ship() else { return };
    let Some(first) = ship.get_first_route_stop() else {
        log_once(u64::from(ship_index) << 32 | 0xffff_fffe, || {
            info!(
                "route load: ship {ship_index} has no first route stop (+0x132 = {:#06x})",
                ship.get_route_stop_index()
            )
        });
        return;
    };

    let (capacity, has_max) = first.load_capacity();
    // The origin is zero on this pass, so the panel's own position is the screen position.
    let x = panel.get_x() + LOAD_X;
    let y = panel.get_y() + LOAD_Y;
    log_once(u64::from(ship_index) << 32 | capacity as u32 as u64, || {
        info!(
            "route load: ship {ship_index} first stop town {} -> {capacity}{} at ({x},{y}) [panel origin {},{} size {}x{} + draw origin {origin_x},{origin_y}]",
            first.get_town_index(),
            if has_max { "+" } else { "" },
            panel.get_x(),
            panel.get_y(),
            panel.get_width(),
            panel.get_height(),
        )
    });
    if capacity == 0 && !has_max {
        return;
    }
    let text = if has_max { format!("{capacity}+") } else { capacity.to_string() };

    // Tiepolo Bold at 16px, the panel's own body face. See `p3_api::ui::font` for all
    // six and what each one is.
    font::ddraw_set_font(font::get_normal_font());
    font::ddraw_set_text_mode(TextMode::AlignRight);
    let mut buffer = text.into_bytes();
    buffer.push(0);

    // The plate first, then the glyphs on top of it. Right-aligned text runs leftwards
    // from `x`, so the box starts a text-width back.
    let width = (buffer.len() as i32 - 1) * CHAR_WIDTH;
    let plate = (x - width - PAD_X, y - PAD_Y, width + PAD_X * 2, LINE_HEIGHT + PAD_Y * 2);
    ddraw_set_constant_color(BACKGROUND);
    ddraw_fill_solid_rect(plate.0, plate.1, plate.2, plate.3);
    ddraw_set_constant_color(COLOR);
    ui_render_text_at(x, y, &buffer);
}

/// Log only when the value changes: this runs on every frame the panel draws.
fn log_once(key: u64, emit: impl FnOnce()) {
    if LAST_LOGGED.swap(key, Ordering::Relaxed) != key {
        emit();
    }
}
