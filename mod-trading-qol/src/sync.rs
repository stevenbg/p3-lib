//! The administrator page's sync column: a checkbox per ware row, left of the ware names,
//! and a "Sync" button in the bottom-left corner - built from the game's own widget classes
//! and living on the game's trading office window. The checked rows are the orders to apply
//! to the player's other trading offices.
//!
//! A checkbox is what the game's own lock checkbox is: a round button and, registered after
//! it so it draws on top, the checkmark image, shown while the row is selected. The widgets
//! are built once (the button templates exist only after startup), registered with the
//! scene on every open of the window after the game's own children, shown only while the
//! administrator page is up, and taken out on close. The window's vtable open/update/close
//! hooks in [crate::ffi] drive [on_open], [on_update] and [on_close].

use std::sync::Mutex;

use log::{info, warn};
use num_traits::FromPrimitive;
use p3_api::{
    data::{ddraw_set_constant_color, enums::WareId, ui_render_text_at},
    game_world::GAME_WORLD_PTR,
    operation::Operation,
    operations::{execute_operation, OPERATIONS_PTR},
    town::get_town_name,
    ui::{
        animation::{Animation2D, LOCK_CHECKMARK_ID, LOCK_CHECKMARK_INI, LOCK_CHECKMARK_X_OFFSET},
        button::{Button, ButtonKind, ButtonTemplate},
        font,
        ui_trading_office_window::{UITradingOfficeWindowPtr, ADMINISTRATOR_PAGE, CORNER_BUTTON_MARGIN, FIRST_ROW_Y, ROW_COUNT, ROW_PITCH, WARE_DISPLAY_ORDER},
    },
};

/// The checkbox column's x inside the window; the checkmark hangs 4 px further left, as
/// the game's does, so the pair spans 2..26 px, short of the ware names.
const COLUMN_X: i32 = 6;
const SYNC_CAPTION: &[u8] = b"Set";
/// The explanation drawn to the right of the Sync button (latin1, NUL-terminated for the
/// game's text renderer), and its gap from the button and offset down to the button's baseline.
const SYNC_HINT: &[u8] = b"stock & price across offices for that order type\0";
const SYNC_HINT_GAP: i32 = 6;
const SYNC_HINT_Y_OFFSET: i32 = 2;
const BLACK: u32 = 0xff00_0000;

struct Widgets {
    checks: Vec<Button>,
    marks: Vec<Animation2D>,
    sync: Button,
    /// Per row, in display order.
    selected: [bool; ROW_COUNT],
    shown: bool,
}

static WIDGETS: Mutex<Option<Widgets>> = Mutex::new(None);

/// Build the widgets on the first open; `None` if the game's button templates are missing.
unsafe fn build() -> Option<Widgets> {
    let mut checks = Vec::with_capacity(ROW_COUNT);
    let mut marks = Vec::with_capacity(ROW_COUNT);
    for _ in 0..ROW_COUNT {
        let check = Button::new(ButtonTemplate::Round)?;
        check.set_kind(ButtonKind::Normal);
        checks.push(check);
        marks.push(Animation2D::new(LOCK_CHECKMARK_INI, LOCK_CHECKMARK_ID));
    }
    let sync = Button::new(ButtonTemplate::Medium)?;
    sync.set_text(SYNC_CAPTION);
    sync.set_kind(ButtonKind::Normal);
    info!("sync column: built {ROW_COUNT} checkboxes and the Sync button");
    Some(Widgets { checks, marks, sync, selected: [false; ROW_COUNT], shown: false })
}

/// Place everything relative to the window's current rect.
unsafe fn place(widgets: &Widgets, window: &UITradingOfficeWindowPtr) {
    let (x, y) = (window.get_x(), window.get_y());
    for row in 0..ROW_COUNT {
        let row_y = y + FIRST_ROW_Y + row as i32 * ROW_PITCH;
        widgets.checks[row].set_position(x + COLUMN_X, row_y);
        widgets.marks[row].set_position(x + COLUMN_X + LOCK_CHECKMARK_X_OFFSET, row_y);
    }
    let (_, sync_height) = widgets.sync.size();
    widgets.sync.set_position(x + CORNER_BUTTON_MARGIN, y + window.get_height() - sync_height - CORNER_BUTTON_MARGIN);
}

unsafe fn show_all(widgets: &Widgets, visible: bool) {
    for row in 0..ROW_COUNT {
        widgets.checks[row].show(visible);
        widgets.marks[row].show(visible && widgets.selected[row]);
    }
    widgets.sync.show(visible);
}

/// After the game's open: register our widgets after its children, hidden - the window
/// opens on the empty page.
pub(crate) unsafe fn on_open(window: &UITradingOfficeWindowPtr) {
    let mut guard = WIDGETS.lock().unwrap();
    if guard.is_none() {
        *guard = build();
        if guard.is_none() {
            warn!("sync column: the button templates are not loaded - no widgets");
            return;
        }
    }
    let widgets = guard.as_mut().unwrap();
    place(widgets, window);
    for row in 0..ROW_COUNT {
        widgets.checks[row].attach_to_root();
        widgets.marks[row].attach_to_root();
    }
    widgets.sync.attach_to_root();
    widgets.shown = false;
    show_all(widgets, false);
}

/// Whether the office shown has an administrator - without one the administrator page
/// shows only the "employ an administrator" notice, and so should we. The index is out of
/// range while nobody is employed, the game's own test.
pub(crate) unsafe fn administrator_employed(window: &UITradingOfficeWindowPtr) -> bool {
    let merchant_index = OPERATIONS_PTR.get_player_merchant_index();
    match GAME_WORLD_PTR.get_office_in_of(window.get_town_index() as _, merchant_index as _) {
        Some(office) => office.get_administrator_index() < p3_api::ships::ShipsPtr::new().get_auto_traders_size(),
        None => false,
    }
}

/// Each frame after the game's update: follow the page, poll the buttons.
pub(crate) unsafe fn on_update(window: &UITradingOfficeWindowPtr) {
    let mut guard = WIDGETS.lock().unwrap();
    let Some(widgets) = guard.as_mut() else { return };
    if !widgets.sync.is_attached() {
        return;
    }
    let on_page = window.get_selected_page() == ADMINISTRATOR_PAGE && administrator_employed(window);
    if on_page != widgets.shown {
        widgets.shown = on_page;
        show_all(widgets, on_page);
    }
    if !on_page {
        return;
    }
    for row in 0..ROW_COUNT {
        if widgets.checks[row].clicked() {
            widgets.selected[row] = !widgets.selected[row];
            widgets.marks[row].show(widgets.selected[row]);
        }
    }
    if widgets.sync.clicked() {
        on_sync(widgets, window);
    }
}

/// After the game's draw: the hint beside the Sync button, while the column is up. The
/// window's own draw has set the render context; we only pick the font, colour and alignment.
pub(crate) unsafe fn on_draw(_window: &UITradingOfficeWindowPtr) {
    let guard = WIDGETS.lock().unwrap();
    let Some(widgets) = guard.as_ref() else { return };
    if !widgets.shown || !widgets.sync.is_attached() {
        return;
    }
    let (x, y) = widgets.sync.position();
    let (width, _) = widgets.sync.size();
    font::ddraw_set_font(font::get_normal_font());
    font::ddraw_set_text_mode(font::TextMode::AlignLeft);
    ddraw_set_constant_color(BLACK);
    ui_render_text_at(x + width + SYNC_HINT_GAP, y + SYNC_HINT_Y_OFFSET, SYNC_HINT);
}

/// After the game's close: out of the scene, and the selection is forgotten - it belongs to
/// the office the window showed, not to the next one.
pub(crate) unsafe fn on_close() {
    let mut guard = WIDGETS.lock().unwrap();
    let Some(widgets) = guard.as_mut() else { return };
    for row in 0..ROW_COUNT {
        widgets.marks[row].detach();
        widgets.checks[row].detach();
    }
    widgets.sync.detach();
    widgets.shown = false;
    widgets.selected = [false; ROW_COUNT];
}

/// The Sync button: copy the selected rows' orders from this office to every other trading
/// office of the player - amount and price, for each ware whose order in the target office
/// runs the same way (a buy stays a buy, a sell a sell). A target with no order for the ware,
/// or one the other way round, is left alone; so is a selected ware without an order here.
/// Executed directly, like the office keys.
unsafe fn on_sync(widgets: &Widgets, window: &UITradingOfficeWindowPtr) {
    let town_index = window.get_town_index() as u8;
    let town = get_town_name(town_index).unwrap_or_else(|| "<unknown>".into());
    let merchant_index = OPERATIONS_PTR.get_player_merchant_index();
    let Some(source) = GAME_WORLD_PTR.get_office_in_of(town_index as _, merchant_index as _) else {
        crate::ffi::notify(&format!("Sync: no player office found in {town}"));
        return;
    };
    let prices = source.get_administrator_trade_prices();
    let stocks = source.get_administrator_trade_stock();

    let mut selected = Vec::new();
    let mut without_order = Vec::new();
    for row in 0..ROW_COUNT {
        if !widgets.selected[row] {
            continue;
        }
        let ware_index = *WARE_DISPLAY_ORDER.add(row) as u16;
        let Some(ware_id) = WareId::from_u16(ware_index).filter(|_| crate::ffi::TRADE_WARES.contains(&ware_index)) else {
            continue;
        };
        // A price of 0 is the game's "no order": nothing to copy.
        if prices[ware_index as usize] == 0 {
            without_order.push(format!("{ware_id:?}"));
        } else {
            selected.push(ware_id);
        }
    }
    if selected.is_empty() {
        crate::ffi::notify(&format!("Sync from {town}: no selected ware has an order here"));
        return;
    }

    let merchant = GAME_WORLD_PTR.get_merchant(merchant_index as _);
    let offices_count = GAME_WORLD_PTR.get_offices_count();
    let mut office_index = merchant.get_first_office_index();
    let mut offices = 0;
    let mut updated = 0;
    let mut skipped = 0;
    let mut visited = 0;
    while office_index < offices_count && visited < offices_count {
        visited += 1;
        let office = GAME_WORLD_PTR.get_office(office_index);
        if office.address != source.address {
            offices += 1;
            let target_prices = office.get_administrator_trade_prices();
            let target_stocks = office.get_administrator_trade_stock();
            for &ware_id in &selected {
                let i = ware_id as usize;
                let (price, stock) = (prices[i], stocks[i]);
                // Same direction = same sign; the target's 0 (no order) never matches.
                if target_prices[i].signum() != price.signum() {
                    skipped += 1;
                    continue;
                }
                if target_prices[i] == price && target_stocks[i] == stock {
                    continue;
                }
                execute_operation(&Operation::OfficeAutotradeSettingChange { stock, price, office_index: office_index as _, ware_id });
                updated += 1;
            }
        }
        office_index = office.get_next_office_of_merchant_index();
    }

    let wares: Vec<String> = selected.iter().map(|ware| format!("{ware:?}")).collect();
    info!(
        "sync from {town}: [{}] to {offices} offices - {updated} orders set, {skipped} skipped for a different order, no order here for [{}]",
        wares.join(", "),
        without_order.join(", ")
    );
    crate::ffi::notify(&format!(
        "Sync from {town}: {updated} orders set in {offices} offices, {skipped} skipped ({} wares)",
        selected.len()
    ));
}
