use std::ffi::CStr;

use p3_api::{
    auto_trader::AutoTraderPtr,
    data::{class48::Class48Ptr, ddraw_set_constant_color, ddraw_set_text_mode, screen_rectangle::Rect, ui_render_text_at},
    game_world::GAME_WORLD_PTR,
    operations::OPERATIONS_PTR,
    ships::ShipsPtr,
    town::get_town_name_bytes,
    ui::{font, rect_clipper_stuff, ui_trading_office_window::UITradingOfficeWindowPtr},
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_F1};

pub static NO_ADMINISTRATOR: &CStr = c"No administrator employed";
pub static CAPTAINS: &CStr = c"Captains in town";
pub static PIRATES: &CStr = c"Pirates in town";
pub static NONE: &CStr = c"none";
pub static NAVIGATION: &CStr = c"Nav";
pub static TRADE: &CStr = c"Trade";
pub static COMBAT: &CStr = c"Comb";
pub static WAGE: &CStr = c"Wage";
pub static SHARE: &CStr = c"Share";
pub static ALL_TOWNS_HINT: &CStr = c"hold F1 for all towns";

/// Text mode 2 draws right-aligned: every column x below is the right edge of that
/// column, so a long town name reaches further left than a short one.
const TOWN_X: i32 = 155;
const NAVIGATION_X: i32 = 200;
const TRADE_X: i32 = 245;
const COMBAT_X: i32 = 295;
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
/// `render_window_title` would spend the top of the window on a banner graphic that
/// the table needs for rows.
pub(crate) unsafe fn draw_page(window: UITradingOfficeWindowPtr) {
    ddraw_set_constant_color(BLACK);
    ddraw_set_text_mode(2);

    let x = window.get_x();
    let mut y = window.get_y() + FIRST_ROW_Y;
    // Stop before the window's bottom edge instead of drawing past it.
    let last_y = window.get_y() + window.get_height() - ROW_HEIGHT;

    y = draw_administrator(x, y, window);
    y += ROW_HEIGHT;

    // Only the towns the player has an office in are of immediate use; holding F1
    // lists every town.
    let all_towns = f1_held();
    let (captains, pirates) = hireable_auto_traders(all_towns);

    y = draw_section(x, y, last_y, CAPTAINS, WAGE, &captains, false);
    y += ROW_HEIGHT;
    y = draw_section(x, y, last_y, PIRATES, SHARE, &pirates, true);

    if !all_towns && y <= last_y {
        font::ddraw_set_font(font::get_normal_font());
        draw_text(x + VALUE_X, y + ROW_HEIGHT, ALL_TOWNS_HINT.to_bytes());
    }
}

fn f1_held() -> bool {
    (unsafe { GetKeyState(VK_F1.0 as i32) } as u16) & 0x8000 != 0
}

/// This office's administrator pays 2% less per trade skill level on everything he
/// buys - an effect the game shows nowhere.
unsafe fn draw_administrator(x: i32, y: i32, window: UITradingOfficeWindowPtr) -> i32 {
    font::ddraw_set_font(font::get_normal_font());
    let town_index = window.get_town_index() as u8;
    let player_merchant = OPERATIONS_PTR.get_player_merchant_index();
    let Some(office) = GAME_WORLD_PTR.get_office_in_of(town_index, player_merchant as _) else {
        return y;
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
    y + ROW_HEIGHT
}

/// One table of auto traders waiting in taverns: a header row whose first cell names
/// what the table lists, then a row per trader. `share` prints the demanded loot
/// share instead of the wage.
unsafe fn draw_section(x: i32, y: i32, last_y: i32, label: &CStr, value_column: &CStr, traders: &[(u8, AutoTraderPtr)], share: bool) -> i32 {
    let mut y = y;
    font::ddraw_set_font(font::get_header_font());
    draw_text(x + TOWN_X, y, label.to_bytes());
    draw_text(x + NAVIGATION_X, y, NAVIGATION.to_bytes());
    draw_text(x + TRADE_X, y, TRADE.to_bytes());
    draw_text(x + COMBAT_X, y, COMBAT.to_bytes());
    draw_text(x + VALUE_X, y, value_column.to_bytes());
    y += ROW_HEIGHT;

    font::ddraw_set_font(font::get_normal_font());
    if traders.is_empty() {
        draw_text(x + TOWN_X, y, NONE.to_bytes());
        return y + ROW_HEIGHT;
    }

    for (town_index, trader) in traders {
        if y > last_y {
            break;
        }
        if let Some(town) = get_town_name_bytes(*town_index) {
            draw_text(x + TOWN_X, y, &town);
        }
        draw_number(x + NAVIGATION_X, y, AutoTraderPtr::skill_level(trader.get_navigation_skill()) as i32, "");
        draw_number(x + TRADE_X, y, AutoTraderPtr::skill_level(trader.get_trade_skill()) as i32, "");
        draw_number(x + COMBAT_X, y, AutoTraderPtr::skill_level(trader.get_combat_skill()) as i32, "");
        if share {
            draw_number(x + VALUE_X, y, trader.get_pirate_loot_share_percent() as i32, " %");
        } else {
            draw_number(x + VALUE_X, y, trader.get_daily_wage() as i32, "");
        }
        y += ROW_HEIGHT;
    }
    y
}

/// The captains and the pirate captains nobody employs, by town: the records chained
/// to a town are the ones sitting in its tavern, and an unemployed one (merchant
/// `0xFF`) is the one its resolver hands out. Unless `all_towns`, only towns the
/// player has a trading office in are considered.
unsafe fn hireable_auto_traders(all_towns: bool) -> (Vec<(u8, AutoTraderPtr)>, Vec<(u8, AutoTraderPtr)>) {
    let ships = ShipsPtr::new();
    let auto_traders = ships.get_auto_traders_size();
    let player_merchant = OPERATIONS_PTR.get_player_merchant_index();
    let mut captains = Vec::new();
    let mut pirates = Vec::new();

    for town_index in 0..GAME_WORLD_PTR.get_towns_count().min(0xff) as u8 {
        if !all_towns && GAME_WORLD_PTR.get_office_in_of(town_index, player_merchant as _).is_none() {
            continue;
        }
        let mut index = GAME_WORLD_PTR.get_town(town_index).get_auto_trader_chain_head();
        // The chain ends on an out-of-range index; the count also caps the walk.
        for _ in 0..auto_traders {
            let Some(trader) = ships.get_auto_trader(index) else { break };
            if trader.get_merchant_index() == 0xff {
                if trader.is_captain() {
                    captains.push((town_index, trader));
                } else {
                    pirates.push((town_index, trader));
                }
            }
            index = trader.get_next_index();
        }
    }

    (captains, pirates)
}

/// The game's text drawing takes a NUL-terminated string in its own codepage, so
/// latin1 bytes go through unchanged.
unsafe fn draw_text(x: i32, y: i32, text: &[u8]) {
    let mut buffer = text.to_vec();
    buffer.push(0);
    ui_render_text_at(x, y, &buffer);
}

unsafe fn draw_number(x: i32, y: i32, value: i32, suffix: &str) {
    draw_text(x, y, format!("{value}{suffix}").as_bytes());
}
