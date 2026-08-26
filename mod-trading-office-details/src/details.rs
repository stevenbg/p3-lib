use std::ffi::CStr;

use p3_api::{
    auto_trader::AutoTraderPtr,
    data::{class48::Class48Ptr, ddraw_set_constant_color, ddraw_set_text_mode, screen_rectangle::Rect, ui_render_text_at},
    facility,
    game_setup,
    game_world::GAME_WORLD_PTR,
    operations::OPERATIONS_PTR,
    ships::ShipsPtr,
    ui::{font, rect_clipper_stuff, ui_trading_office_window::UITradingOfficeWindowPtr},
};

pub static NO_ADMINISTRATOR: &CStr = c"No administrator employed";

/// Text mode 2 draws right-aligned, so this x is the line's right edge.
const VALUE_X: i32 = 345;
const FIRST_ROW_Y: i32 = 12;
const ROW_HEIGHT: i32 = 16;
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

    let x = window.get_x();
    let mut y = window.get_y() + FIRST_ROW_Y;
    draw_administrator(x, y, window);
    y += ROW_HEIGHT;
    y = draw_pirates(x, y);
    draw_winter(x, y, window.get_town_index() as u8);
}

/// The two pirate facts the game never states, both driven by the single "Pirates activity"
/// setting rather than by the difficulty preset (which merely writes that setting along with
/// every other one).
unsafe fn draw_pirates(x: i32, y: i32) -> i32 {
    font::ddraw_set_font(font::get_normal_font());
    let mut y = y;

    let Some(activity) = game_setup::get_pirate_activity() else {
        return y;
    };
    let level = match activity {
        0 => "low",
        1 => "normal",
        2 => "high",
        _ => "?",
    };

    // Band count is a world-generation figure: 2 * activity + 1 bands were created when the
    // game started, and changing the setting later cannot add or remove any.
    if let Some(bands) = game_setup::pirate_band_count() {
        let line = format!("Pirate bands roaming: {bands} (activity {level})");
        draw_text(x + VALUE_X, y, line.as_bytes());
        y += ROW_HEIGHT;
    }

    // A free pirate robs a merchant once `rank_in_home_town + activity >= 2`, so the higher
    // the setting the lower the rank it settles for. Showing the player's own rank next to
    // the threshold makes it clear whether he is already fair game.
    if let Some(threshold) = game_setup::pirate_attack_rank_threshold() {
        let merchant = GAME_WORLD_PTR.get_merchant(OPERATIONS_PTR.get_player_merchant_index() as u16);
        let rank = merchant.get_rank_in(merchant.get_hometown_index());
        let verdict = if rank >= threshold { "you qualify" } else { "you are beneath notice" };
        let line = format!("Pirates rob from home rank {threshold} (yours {rank}: {verdict})");
        draw_text(x + VALUE_X, y, line.as_bytes());
        y += ROW_HEIGHT;
    }
    y
}

/// Which months carry the crop penalty, whether one of them is running, and what it costs.
/// The game shows none of this, and the size of it is worth knowing before planning a
/// farming town's supply.
unsafe fn draw_winter(x: i32, y: i32, town_index: u8) -> i32 {
    font::ddraw_set_font(font::get_normal_font());
    let mut y = y;

    // The month list is `GameWorldPtr::WINTER_MONTHS` (11, 0, 1 - the field is zero-based).
    let now = if GAME_WORLD_PTR.is_winter() { " - in force now" } else { "" };
    draw_text(x + VALUE_X, y, format!("Crop winter: Dec, Jan, Feb{now}").as_bytes());
    y += ROW_HEIGHT;

    // Computed from this town's own flag word rather than hardcoded, so the line stays
    // honest if the two unidentified crop bits ever turn out to be set somewhere: each
    // producer's ladder is asked for its factor with the winter bit forced off and forced
    // on, and the ratio of the two is the penalty.
    let flags = GAME_WORLD_PTR.get_town(town_index).get_flags();
    let mut by_percent: Vec<(u32, Vec<&str>)> = Vec::new();
    for (ware, name) in facility::CROP_WARES.iter().zip(["grain", "honey", "wine", "hemp"]) {
        let Some(percent) = facility::crop_winter_percent(*ware, flags) else {
            continue;
        };
        match by_percent.iter_mut().find(|(p, _)| *p == percent) {
            Some((_, names)) => names.push(name),
            None => by_percent.push((percent, vec![name])),
        }
    }
    if !by_percent.is_empty() {
        // Wares that lose the same share share a term, which is what keeps the line short
        // enough for the window: "grain 66%, honey/wine/hemp 50%".
        let parts: Vec<String> = by_percent
            .iter()
            .map(|(percent, names)| format!("{} {percent}%", names.join("/")))
            .collect();
        draw_text(x + VALUE_X, y, format!("Winter output: {}", parts.join(", ")).as_bytes());
        y += ROW_HEIGHT;
    }
    y
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
