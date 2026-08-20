use std::ffi::{CStr, CString};

use p3_api::{
    data::{
        class48::Class48Ptr, ddraw_set_constant_color, ddraw_set_text_mode, fill_p3_string, render_window_title, screen_rectangle::Rect,
        ui_render_text_at,
    },
    game_world::GAME_WORLD_PTR,
    operations::OPERATIONS_PTR,
    ships::ShipsPtr,
    ui::{font, rect_clipper_stuff, ui_trading_office_window::UITradingOfficeWindowPtr},
};

pub static TITLE: &CStr = c"Details";
pub static BUYING_DISCOUNT: &CStr = c"Buying discount";
pub static NO_ADMINISTRATOR: &CStr = c"no administrator";

const LABEL_X: i32 = 200;
const VALUE_X: i32 = 340;
const ROW_Y: i32 = 200;
const BLACK: u32 = 0xff000000;

/// Called when the window opens, like the other details mods do it, so the page's
/// text is not clipped away.
pub(crate) unsafe fn prepare_drawing_state() {
    let class48 = Class48Ptr::new();
    class48.set_ignore_below_gradient(0);
    class48.set_gradient_y(0);
}

/// Submit the window's area to the renderer. Called from the window's update
/// method, which is the phase the game itself uses for this (the town hall window
/// does it there, gated by the day timestamp at `+0x1930`); called from the draw
/// method instead - once or per frame - the art comes out torn and the text
/// flickers.
pub(crate) unsafe fn invalidate(window: UITradingOfficeWindowPtr) {
    let rect = Rect {
        left: window.get_x(),
        top: window.get_y(),
        right: window.get_x() + window.get_width(),
        bottom: window.get_y() + window.get_height(),
    };
    rect_clipper_stuff(&rect);
}

pub(crate) unsafe fn draw_page(window: UITradingOfficeWindowPtr) {
    let mut title_p3_string: u32 = 0;
    fill_p3_string((&mut title_p3_string) as *mut _ as _, TITLE.to_bytes());
    render_window_title(title_p3_string as _, window.address as _);

    ddraw_set_constant_color(BLACK);
    ddraw_set_text_mode(2);
    font::ddraw_set_font(font::get_normal_font());

    let x = window.get_x();
    let y = window.get_y() + ROW_Y;
    let town_index = window.get_town_index() as u8;
    let player_merchant = OPERATIONS_PTR.get_player_merchant_index();
    let Some(office) = GAME_WORLD_PTR.get_office_in_of(town_index, player_merchant as _) else {
        return;
    };

    ui_render_text_at(x + LABEL_X, y, BUYING_DISCOUNT.to_bytes());
    // The administrator's trade skill makes him pay 2% less per level on everything
    // he buys - an effect the game shows nowhere.
    match ShipsPtr::new().get_auto_trader(office.get_administrator_index()) {
        None => ui_render_text_at(x + VALUE_X, y, NO_ADMINISTRATOR.to_bytes()),
        Some(administrator) => {
            let discount = 100 - administrator.get_buy_percent_paid();
            if let Ok(value) = CString::new(format!("{discount} %")) {
                ui_render_text_at(x + VALUE_X, y, value.to_bytes());
            }
        }
    }
}
