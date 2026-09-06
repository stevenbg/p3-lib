//! The administrator page's locked-amount column: a number box per ware row in the room
//! between the amount `+` and the lock checkbox that [crate::office_layout] makes. Built
//! from the game's own box class the way the office builds its amount and price boxes, so
//! it looks the same, draws itself and takes the keyboard when clicked.
//!
//! The amounts live in [STORE], one row of raw units per town the player has an office in
//! (the same units as the office's minimum store, `office + 0x354`: 200 per barrel, 2000
//! per load), keyed by town because a merchant has at most one office per town. The boxes
//! are filled from the store when the window opens and written back when it closes; the
//! store is what a save sidecar will serialize. Nothing enforces the amounts yet
//! (`.claude/notes/todo/office-locked-amounts.md`). The window's vtable open/update/close
//! hooks in [crate::ffi] drive [on_open], [on_update] and [on_close], as they do the sync
//! column.

use std::{collections::HashMap, sync::Mutex};

use log::{debug, info};
use num_traits::FromPrimitive;
use p3_api::{
    data::enums::WareId,
    ui::{
        number_widget::{NumberWidget, OFFICE_BOX_ID, OFFICE_BOX_INI, OFFICE_BOX_MAX},
        ui_trading_office_window::{
            UITradingOfficeWindowPtr, ADMINISTRATOR_PAGE, AMOUNT_PLUS_BUTTONS_OFFSET, AMOUNT_WIDGETS_OFFSET, BUTTON_SIZE, FIRST_ROW_Y, IMAGE_SIZE,
            LOCK_CHECKMARKS_OFFSET, ROW_COUNT, ROW_PITCH, WARE_DISPLAY_ORDER,
        },
        widget,
    },
};

/// From the amount `+` button's right edge to the box: the game's own spacing between
/// neighbours in the row.
const GAP: i32 = 3;

/// The locked amounts, raw units, indexed by ware id; one row per town index.
pub(crate) type TownAmounts = [i32; ROW_COUNT];

/// Every office's locked amounts, by town. A town without an entry has none locked.
pub(crate) static STORE: Mutex<Option<HashMap<u8, TownAmounts>>> = Mutex::new(None);

pub(crate) fn amounts_of(town: u8) -> TownAmounts {
    STORE.lock().unwrap().as_ref().and_then(|store| store.get(&town).copied()).unwrap_or([0; ROW_COUNT])
}

/// Remember a town's amounts; all zero forgets the town.
pub(crate) fn set_amounts(town: u8, amounts: TownAmounts) {
    let mut guard = STORE.lock().unwrap();
    let store = guard.get_or_insert_with(HashMap::new);
    if amounts.iter().all(|&a| a == 0) {
        store.remove(&town);
    } else {
        store.insert(town, amounts);
    }
}

struct Widgets {
    boxes: Vec<NumberWidget>,
    shown: bool,
    /// Each row's lock state as of the last frame, to catch the click that locks a ware.
    locked: [bool; ROW_COUNT],
}

/// Whether the game shows the row's lock checkmark - the ware is locked.
unsafe fn is_locked(window: &UITradingOfficeWindowPtr, row: usize) -> bool {
    widget::is_visible(window.address + LOCK_CHECKMARKS_OFFSET + row as u32 * IMAGE_SIZE)
}

/// The row's stock amount box, the game's own.
fn amount_box(window: &UITradingOfficeWindowPtr, row: usize) -> NumberWidget {
    NumberWidget::new(window.address + AMOUNT_WIDGETS_OFFSET + row as u32 * p3_api::ui::number_widget::OBJECT_SIZE)
}

static WIDGETS: Mutex<Option<Widgets>> = Mutex::new(None);

/// The ware a row shows, in the window's display order.
unsafe fn ware_of_row(row: usize) -> Option<WareId> {
    WareId::from_u8(*WARE_DISPLAY_ORDER.add(row))
}

/// Fill the boxes from the store, in the boxes' display units.
unsafe fn load(widgets: &Widgets, town: u8) {
    let amounts = amounts_of(town);
    for row in 0..ROW_COUNT {
        let value = match ware_of_row(row) {
            Some(ware) => amounts[ware as usize] / ware.get_scaling(),
            None => 0,
        };
        widgets.boxes[row].set_value(value);
    }
}

/// Write the boxes back to the store, in raw units.
unsafe fn save(widgets: &Widgets, town: u8) {
    let mut amounts = amounts_of(town);
    for row in 0..ROW_COUNT {
        if let Some(ware) = ware_of_row(row) {
            amounts[ware as usize] = widgets.boxes[row].value() * ware.get_scaling();
        }
    }
    debug!("locked amounts: town {town} -> {amounts:?}");
    set_amounts(town, amounts);
}

unsafe fn build() -> Widgets {
    let boxes = (0..ROW_COUNT).map(|_| NumberWidget::build(OFFICE_BOX_INI, OFFICE_BOX_ID, OFFICE_BOX_MAX)).collect();
    info!("locked amounts: built {ROW_COUNT} boxes");
    Widgets { boxes, shown: false, locked: [false; ROW_COUNT] }
}

/// Place every box right of its row's amount `+` button, on the row's line.
unsafe fn place(widgets: &Widgets, window: &UITradingOfficeWindowPtr) {
    let y = window.get_y();
    for row in 0..ROW_COUNT {
        let plus = window.address + AMOUNT_PLUS_BUTTONS_OFFSET + row as u32 * BUTTON_SIZE;
        let (plus_x, _) = widget::position(plus);
        let (plus_width, _) = widget::size(plus);
        widgets.boxes[row].set_position(plus_x + plus_width + GAP, y + FIRST_ROW_Y + row as i32 * ROW_PITCH);
    }
}

unsafe fn show_all(widgets: &Widgets, visible: bool) {
    for widget in &widgets.boxes {
        widget.show(visible);
    }
}

/// After the game's open: register the boxes after its children, hidden - the window opens
/// on the empty page - and fill them with this town's amounts.
pub(crate) unsafe fn on_open(window: &UITradingOfficeWindowPtr) {
    let mut guard = WIDGETS.lock().unwrap();
    let widgets = guard.get_or_insert_with(|| build());
    place(widgets, window);
    for widget in &widgets.boxes {
        widget.attach_to_root();
    }
    widgets.shown = false;
    show_all(widgets, false);
    load(widgets, window.get_town_index() as u8);
}

/// Each frame after the game's update: follow the page, and when a click has just locked a
/// ware whose locked amount is 0, start it at the row's stock amount - the whole minimum
/// store, what the lock alone would protect.
pub(crate) unsafe fn on_update(window: &UITradingOfficeWindowPtr) {
    let mut guard = WIDGETS.lock().unwrap();
    let Some(widgets) = guard.as_mut() else { return };
    if !widgets.boxes[0].is_attached() {
        return;
    }
    let on_page = window.get_selected_page() == ADMINISTRATOR_PAGE && crate::sync::administrator_employed(window);
    if on_page != widgets.shown {
        widgets.shown = on_page;
        show_all(widgets, on_page);
        // Entering the page: take the locks as they are, without copying.
        for row in 0..ROW_COUNT {
            widgets.locked[row] = is_locked(window, row);
        }
    }
    if !on_page {
        return;
    }
    for row in 0..ROW_COUNT {
        let locked = is_locked(window, row);
        if locked && !widgets.locked[row] && widgets.boxes[row].value() == 0 {
            widgets.boxes[row].set_value(amount_box(window, row).value());
        }
        widgets.locked[row] = locked;
    }
}

/// After the game's close: the boxes' values back to the store, then out of the scene.
pub(crate) unsafe fn on_close(window: &UITradingOfficeWindowPtr) {
    let mut guard = WIDGETS.lock().unwrap();
    let Some(widgets) = guard.as_mut() else { return };
    if widgets.boxes[0].is_attached() {
        save(widgets, window.get_town_index() as u8);
    }
    for widget in &widgets.boxes {
        widget.detach();
    }
    widgets.shown = false;
}
