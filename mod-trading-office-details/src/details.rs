use std::ffi::CStr;

use p3_api::{
    auto_trader::AutoTraderPtr,
    data::{class48::Class48Ptr, ddraw_set_constant_color, ddraw_set_text_mode, screen_rectangle::Rect, ui_render_text_at},
    game_world::GAME_WORLD_PTR,
    operations::OPERATIONS_PTR,
    ships::ShipsPtr,
    ui::{font, rect_clipper_stuff, ui_trading_office_window::UITradingOfficeWindowPtr},
};

pub static NO_ADMINISTRATOR: &CStr = c"No administrator employed";

/// Text mode 2 draws right-aligned, so this x is the line's right edge.
const VALUE_X: i32 = 345;
const FIRST_ROW_Y: i32 = 12;
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

/// No window title: the page the game itself draws here has none, and
/// `render_window_title` would spend the top of the window on a banner graphic.
pub(crate) unsafe fn draw_page(window: UITradingOfficeWindowPtr) {
    ddraw_set_constant_color(BLACK);
    ddraw_set_text_mode(2);

    draw_administrator(window.get_x(), window.get_y() + FIRST_ROW_Y, window);
}

/// This office's administrator pays 2% less per trade skill level on everything he
/// buys - an effect the game shows nowhere.
unsafe fn draw_administrator(x: i32, y: i32, window: UITradingOfficeWindowPtr) {
    font::ddraw_set_font(font::get_normal_font());
    let town_index = window.get_town_index() as u8;
    let player_merchant = OPERATIONS_PTR.get_player_merchant_index();
    let Some(office) = GAME_WORLD_PTR.get_office_in_of(town_index, player_merchant as _) else {
        return;
    };

    match ShipsPtr::new().get_auto_trader(office.get_administrator_index()) {
        None => draw_text(x + VALUE_X, y, NO_ADMINISTRATOR.to_bytes()),
        Some(administrator) => {
            let level = AutoTraderPtr::skill_level(administrator.get_trade_skill());
            let discount = 100 - administrator.get_buy_percent_paid();
            let line = format!("Administrator's buying discount: {discount}%, level {level}");
            draw_text(x + VALUE_X, y, line.as_bytes());
        }
    }
}

/// The game's text drawing takes a NUL-terminated string in its own codepage, so
/// latin1 bytes go through unchanged.
unsafe fn draw_text(x: i32, y: i32, text: &[u8]) {
    let mut buffer = text.to_vec();
    buffer.push(0);
    ui_render_text_at(x, y, &buffer);
}
