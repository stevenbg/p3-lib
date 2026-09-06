//! The administrator page's locked-amount column: a number box per ware row in the room
//! between the amount `+` and the lock checkbox that [crate::office_layout] makes. Built
//! from the game's own box class the way the office builds its amount and price boxes, so
//! it looks the same, draws itself and takes the keyboard when clicked.
//!
//! The amounts live in [STORE], one row of raw units per town the player has an office in
//! (the same units as the office's minimum store, `office + 0x354`: 200 per barrel, 2000
//! per load), keyed by town because a merchant has at most one office per town. The boxes
//! are filled from the store when the window opens and written back when it closes; the
//! store travels with the save in [crate::sidecar]. Nothing enforces the amounts yet
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
            UITradingOfficeWindowPtr, ADMINISTRATOR_PAGE, FIRST_ROW_Y, IMAGE_SIZE, LOCK_CHECKMARKS_OFFSET, ROW_COUNT, ROW_PITCH, WARE_DISPLAY_ORDER,
        },
        widget,
    },
};

/// From the box's right edge to the lock checkbox on its right: the game's own spacing
/// between neighbours in the row.
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

/// Every town with locked amounts, in town order - what the save sidecar writes.
pub(crate) fn snapshot() -> Vec<(u8, TownAmounts)> {
    let guard = STORE.lock().unwrap();
    let mut entries: Vec<(u8, TownAmounts)> = guard
        .as_ref()
        .map(|store| store.iter().map(|(&town, &amounts)| (town, amounts)).collect())
        .unwrap_or_default();
    entries.sort_by_key(|&(town, _)| town);
    entries
}

/// Forget every town: the world in memory has been replaced.
pub(crate) fn clear() {
    *STORE.lock().unwrap() = None;
}

/// The whole store at once - what the save sidecar read.
pub(crate) fn replace(entries: HashMap<u8, TownAmounts>) {
    *STORE.lock().unwrap() = Some(entries);
}

/// The open window's boxes into the store now, without waiting for the window to close -
/// so a save carries what the boxes show. Nothing when no office window is open.
pub(crate) unsafe fn flush_open_window() {
    let guard = WIDGETS.lock().unwrap();
    let Some(widgets) = guard.as_ref() else { return };
    if widgets.boxes[0].is_attached() {
        save(widgets, widgets.town);
    }
}

struct Widgets {
    boxes: Vec<NumberWidget>,
    shown: bool,
    /// The town of the window the boxes are attached to.
    town: u8,
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
    Widgets { boxes, shown: false, town: 0 }
}

/// Place every box just left of its row's lock checkbox, on the row's line. The checkmark
/// image is the checkbox's leftmost part (it hangs left of the round button), so the box's
/// right edge sits [GAP] px left of it.
unsafe fn place(widgets: &Widgets, window: &UITradingOfficeWindowPtr) {
    let y = window.get_y();
    for row in 0..ROW_COUNT {
        let checkmark = window.address + LOCK_CHECKMARKS_OFFSET + row as u32 * IMAGE_SIZE;
        let (checkmark_x, _) = widget::position(checkmark);
        let (box_width, _) = widgets.boxes[row].size();
        widgets.boxes[row].set_position(checkmark_x - GAP - box_width, y + FIRST_ROW_Y + row as i32 * ROW_PITCH);
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
    widgets.town = window.get_town_index() as u8;
    show_all(widgets, false);
    load(widgets, widgets.town);
}

/// Each frame after the game's update: show the boxes only on the administrator page with an
/// administrator employed, hide them elsewhere.
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
