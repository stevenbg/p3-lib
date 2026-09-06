//! The administrator page's row layout: the price column against the window's right edge,
//! the lock checkbox next to it, no "min" / "max" labels - and so a free stretch between
//! the amount `+` and the lock for a column of the mod's own. Applied from
//! [crate::ffi::start].
//!
//! The open method places the price `+` button's right edge 25 px inside the window and
//! lays the rest of the row out leftwards from it, each widget its width plus a fixed gap
//! from the previous. Three immediates re-lay the row without moving the amount widgets or
//! the arrows: the right margin, and the two gaps on either side of the lock button, whose
//! sum grows by what the margin gave up. The labels go through one text-render call in the
//! page draw, replaced by `nop`s; its arguments are pushed and cleaned by the caller, so
//! the stack stays balanced, and the strings stay for the goods dialog, which shares them.
//!
//! The lock checkbox's own tooltip is rewritten here too - the box beside it has no tooltip
//! of its own (a number box's class carries no tooltip method), so its neighbour speaks for
//! it. See [p3_api::ui::tooltip].

use p3_api::{
    memory::write_readonly,
    ui::{
        tooltip::LOCK_CHECKBOX_TOOLTIP,
        ui_trading_office_window::{
            CORNER_BUTTON_MARGIN, LABEL_RENDER_CALL_ORIGINAL, LABEL_RENDER_CALL_SITE, ROW_AMOUNT_GAP_ORIGINAL, ROW_AMOUNT_GAP_SITE, ROW_LOCK_GAP_ORIGINAL,
            ROW_LOCK_GAP_SITE, ROW_RIGHT_MARGIN_ORIGINAL, ROW_RIGHT_MARGIN_SITE,
        },
    },
};

/// The lock checkbox's tooltip, hovered next to the mod's locked-amount box. At most
/// [`LOCK_CHECKBOX_TOOLTIP.capacity`](p3_api::ui::tooltip::StringResource::capacity) = 45
/// latin1 characters, the length of the string it replaces.
const LOCK_TOOLTIP_TEXT: &str = "Held back from trade routes. 0 = stock";

/// The price `+` button's right edge, from the window's right edge: the same margin the
/// close button keeps, the least the wooden frame allows.
const PRICE_RIGHT_MARGIN: i8 = CORNER_BUTTON_MARGIN as i8;
/// Between the lock button and the price `-`: the game's own spacing between neighbours.
const LOCK_TO_PRICE_GAP: u8 = ROW_AMOUNT_GAP_ORIGINAL;

pub(crate) unsafe fn install() -> Result<(), &'static str> {
    let game_margin = -(ROW_RIGHT_MARGIN_ORIGINAL as i8);
    let gained = (game_margin - PRICE_RIGHT_MARGIN) as u8;
    // The room between the amount group and the price group, minus the lock button's own
    // spacing, all goes to the amount side.
    let amount_to_lock_gap = ROW_LOCK_GAP_ORIGINAL + ROW_AMOUNT_GAP_ORIGINAL + gained - LOCK_TO_PRICE_GAP;
    patch(ROW_RIGHT_MARGIN_SITE, &[ROW_RIGHT_MARGIN_ORIGINAL], &[(-PRICE_RIGHT_MARGIN) as u8])?;
    patch(ROW_LOCK_GAP_SITE, &[ROW_LOCK_GAP_ORIGINAL], &[LOCK_TO_PRICE_GAP])?;
    patch(ROW_AMOUNT_GAP_SITE, &[ROW_AMOUNT_GAP_ORIGINAL], &[amount_to_lock_gap])?;
    patch(LABEL_RENDER_CALL_SITE, &LABEL_RENDER_CALL_ORIGINAL, &[0x90; 5])?;
    LOCK_CHECKBOX_TOOLTIP.overwrite(LOCK_TOOLTIP_TEXT)?;
    Ok(())
}

/// Write `new` over `expected` at `address`, refusing when the bytes there are not the
/// expected ones - a different game build fails loudly instead of corrupting code.
unsafe fn patch(address: u32, expected: &[u8], new: &[u8]) -> Result<(), &'static str> {
    let found = std::slice::from_raw_parts(address as *const u8, expected.len());
    if found != expected {
        log::error!("unexpected bytes at {address:#010x}: {found:02x?}, expected {expected:02x?} - not patching");
        return Err("unexpected bytes at a layout patch site");
    }
    write_readonly(address, new)
}
