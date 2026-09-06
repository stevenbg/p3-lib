//! The administrator page's locked-amount column: a number box per ware row in the room
//! between the amount `+` and the lock checkbox that [crate::office_layout] makes. Built
//! from the game's own box class the way the office builds its amount and price boxes, so
//! it looks the same, draws itself and takes the keyboard when clicked.
//!
//! For now the boxes only take input: what they hold is not yet stored with the save or
//! enforced against the auto-trade ships (`.claude/notes/todo/office-locked-amounts.md`).
//! The window's vtable open/update/close hooks in [crate::ffi] drive [on_open], [on_update]
//! and [on_close], as they do the sync column.

use std::sync::Mutex;

use log::info;
use p3_api::ui::{
    number_widget::{NumberWidget, OFFICE_BOX_ID, OFFICE_BOX_INI, OFFICE_BOX_MAX},
    ui_trading_office_window::{UITradingOfficeWindowPtr, ADMINISTRATOR_PAGE, AMOUNT_PLUS_BUTTONS_OFFSET, BUTTON_SIZE, FIRST_ROW_Y, ROW_COUNT, ROW_PITCH},
    widget,
};

/// From the amount `+` button's right edge to the box: the game's own spacing between
/// neighbours in the row.
const GAP: i32 = 3;

struct Widgets {
    boxes: Vec<NumberWidget>,
    shown: bool,
}

static WIDGETS: Mutex<Option<Widgets>> = Mutex::new(None);

unsafe fn build() -> Widgets {
    let boxes = (0..ROW_COUNT).map(|_| NumberWidget::build(OFFICE_BOX_INI, OFFICE_BOX_ID, OFFICE_BOX_MAX)).collect();
    info!("locked amounts: built {ROW_COUNT} boxes");
    Widgets { boxes, shown: false }
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
/// on the empty page.
pub(crate) unsafe fn on_open(window: &UITradingOfficeWindowPtr) {
    let mut guard = WIDGETS.lock().unwrap();
    let widgets = guard.get_or_insert_with(|| build());
    place(widgets, window);
    for widget in &widgets.boxes {
        widget.attach_to_root();
    }
    widgets.shown = false;
    show_all(widgets, false);
}

/// Each frame after the game's update: follow the page.
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

/// After the game's close: out of the scene.
pub(crate) unsafe fn on_close() {
    let mut guard = WIDGETS.lock().unwrap();
    let Some(widgets) = guard.as_mut() else { return };
    for widget in &widgets.boxes {
        widget.detach();
    }
    widgets.shown = false;
}
