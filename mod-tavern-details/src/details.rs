use std::ffi::CStr;
use std::sync::atomic::{AtomicBool, Ordering};

use p3_api::{
    auto_trader::AutoTraderPtr,
    data::{class48::Class48Ptr, ddraw_set_constant_color, ddraw_set_text_mode, screen_rectangle::Rect, ui_render_text_at},
    game_world::GAME_WORLD_PTR,
    operations::OPERATIONS_PTR,
    ships::ShipsPtr,
    town::get_town_name_bytes,
    ui::{font, rect_clipper_stuff, ui_tavern_window::UITavernWindowPtr},
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VIRTUAL_KEY, VK_F1, VK_F2, VK_MENU};

pub static CAPTAINS: &CStr = c"Captains in town";
pub static PIRATES: &CStr = c"Pirates in town";
pub static SAILORS: &CStr = c"Sailors in town";
/// The filtered views only cover the towns the player may enter, so their headings say
/// "Known": what they list is what he can see, not what exists.
pub static KNOWN_CAPTAINS: &CStr = c"Known captains in town";
pub static KNOWN_PIRATES: &CStr = c"Known pirates in town";
pub static KNOWN_SAILORS: &CStr = c"Known sailors in town";
pub static NONE: &CStr = c"none";
pub static NAVIGATION: &CStr = c"Nav";
pub static TRADE: &CStr = c"Trade";
pub static COMBAT: &CStr = c"Comb";
pub static WAGE: &CStr = c"Wage";
pub static SHARE: &CStr = c"Share";
pub static AVAILABLE: &CStr = c"Available";
pub static SAILORS_HINT: &CStr = c"F2: sailors";
pub static CAPTAINS_HINT: &CStr = c"F1: captains";

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

/// What the page shows, switched by a key press rather than held, and kept across
/// openings of the window: F1 the captains and pirates, F2 the sailor pools, with alt
/// selecting every town instead of only the enterable ones.
static SHOW_SAILORS: AtomicBool = AtomicBool::new(false);
static SHOW_ALL_TOWNS: AtomicBool = AtomicBool::new(false);
static F1_WAS_DOWN: AtomicBool = AtomicBool::new(false);
static F2_WAS_DOWN: AtomicBool = AtomicBool::new(false);

/// Read the page's keys, called once per frame from the update phase and only while the
/// page is actually on screen. That is the whole scoping: the F1 that mod-auto-supply
/// binds in the trading office is untouched anywhere but on this page, and no global
/// keyboard hook is involved. Acts on the down edge, so a key switches the view rather
/// than needing to be held.
pub(crate) fn poll_keys() {
    if key_pressed(VK_F1, &F1_WAS_DOWN) {
        SHOW_SAILORS.store(false, Ordering::Relaxed);
        SHOW_ALL_TOWNS.store(key_down(VK_MENU), Ordering::Relaxed);
    }
    if key_pressed(VK_F2, &F2_WAS_DOWN) {
        SHOW_SAILORS.store(true, Ordering::Relaxed);
        SHOW_ALL_TOWNS.store(key_down(VK_MENU), Ordering::Relaxed);
    }
}

/// True on the frame the key goes down, `was_down` carrying the previous state. The
/// store has to happen on every call, release included - short-circuiting it away
/// leaves `was_down` stuck at true and the key works exactly once.
fn key_pressed(key: VIRTUAL_KEY, was_down: &AtomicBool) -> bool {
    let down = key_down(key);
    let previously_down = was_down.swap(down, Ordering::Relaxed);
    down && !previously_down
}

/// Called when the window opens, like the other details mods do it, so the page's
/// text is not clipped away.
pub(crate) unsafe fn prepare_drawing_state() {
    let class48 = Class48Ptr::new();
    class48.set_ignore_below_gradient(0);
    class48.set_gradient_y(0);
}

/// Submit the window's area to the renderer. Called from the window's update method
/// (`0x005CD540`), which is the phase the game itself uses for this call; from the
/// draw method instead - once or per frame - the art comes out torn and the text
/// flickers.
pub(crate) unsafe fn invalidate(window: UITavernWindowPtr) {
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
/// the tables need for rows.
pub(crate) unsafe fn draw_page(window: UITavernWindowPtr) {
    ddraw_set_constant_color(BLACK);
    ddraw_set_text_mode(2);

    let x = window.get_x();
    let mut y = window.get_y() + FIRST_ROW_Y;
    // Stop before the window's bottom edge instead of drawing past it.
    let last_y = window.get_y() + window.get_height() - ROW_HEIGHT;

    let all_towns = SHOW_ALL_TOWNS.load(Ordering::Relaxed);
    let sailors = SHOW_SAILORS.load(Ordering::Relaxed);
    let towns = enterable_towns(all_towns);

    if sailors {
        let heading = if all_towns { SAILORS } else { KNOWN_SAILORS };
        y = draw_sailors(x, y, last_y, heading, &towns);
    } else {
        let (captains, pirates) = hireable_auto_traders(&towns);
        let (captains_heading, pirates_heading) = if all_towns { (CAPTAINS, PIRATES) } else { (KNOWN_CAPTAINS, KNOWN_PIRATES) };
        y = draw_section(x, y, last_y, captains_heading, WAGE, &captains, false);
        y += ROW_HEIGHT;
        y = draw_section(x, y, last_y, pirates_heading, SHARE, &pirates, true);
    }

    if y <= last_y {
        font::ddraw_set_font(font::get_normal_font());
        let other_view = if sailors { CAPTAINS_HINT } else { SAILORS_HINT };
        draw_text(x + VALUE_X, y + ROW_HEIGHT, other_view.to_bytes());
    }
}

fn key_down(key: VIRTUAL_KEY) -> bool {
    (unsafe { GetKeyState(key.0 as i32) } as u16) & 0x8000 != 0
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

/// One row per town: the sailors the player can hire there right now, straight from the
/// number the tavern itself works from.
unsafe fn draw_sailors(x: i32, y: i32, last_y: i32, heading: &CStr, towns: &[u8]) -> i32 {
    let mut y = y;
    font::ddraw_set_font(font::get_header_font());
    draw_text(x + TOWN_X, y, heading.to_bytes());
    draw_text(x + TRADE_X, y, AVAILABLE.to_bytes());
    y += ROW_HEIGHT;

    font::ddraw_set_font(font::get_normal_font());
    let merchant = GAME_WORLD_PTR.get_merchant(OPERATIONS_PTR.get_player_merchant_index() as u16);
    for town_index in towns {
        if y > last_y {
            break;
        }
        if let Some(town) = get_town_name_bytes(*town_index) {
            draw_text(x + TOWN_X, y, &town);
        }
        draw_number(x + TRADE_X, y, merchant.get_available_sailors(*town_index) as i32, "");
        y += ROW_HEIGHT;
    }
    y
}

/// The towns the player may legally enter, which is where he can hire: the ones he has
/// a trading office in, plus the ones one of his ships is in port at. With `all_towns`,
/// every town instead, enterable or not.
unsafe fn enterable_towns(all_towns: bool) -> Vec<u8> {
    let player_merchant = OPERATIONS_PTR.get_player_merchant_index() as u16;
    // Only the filtered view needs the ship set, so the all-towns view skips the walk.
    let ship_towns = if all_towns {
        [false; 0x100]
    } else {
        towns_with_player_ships(&ShipsPtr::new(), player_merchant)
    };

    (0..GAME_WORLD_PTR.get_towns_count().min(0xff) as u8)
        .filter(|&town_index| {
            // The office chain of one town is a handful of records; the ship set is
            // already built, so the cheap test goes first.
            all_towns || ship_towns[town_index as usize] || GAME_WORLD_PTR.get_office_in_of(town_index, player_merchant).is_some()
        })
        .collect()
}

/// The captains and the pirate captains nobody employs, by town: the records chained
/// to a town are the ones sitting in its tavern, and an unemployed one (merchant
/// `0xFF`) is the one its resolver hands out.
unsafe fn hireable_auto_traders(towns: &[u8]) -> (Vec<(u8, AutoTraderPtr)>, Vec<(u8, AutoTraderPtr)>) {
    let ships = ShipsPtr::new();
    let auto_traders = ships.get_auto_traders_size();
    let mut captains = Vec::new();
    let mut pirates = Vec::new();

    for &town_index in towns {
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

/// The towns the player's ships are in, indexed by town.
///
/// Walks the player's OWN ship chain - the merchant record's `+0xE` is its head and
/// every ship's `+0x4` the next link - so the cost is his ship count, not the world's.
/// This is the game's own iteration: the per-merchant ship census at `0x004F0AB1`
/// fetches the merchant record, takes `+0xE`, and follows `+0x4` while the index stays
/// below the ship count.
///
/// `is_in_port` is the game's own "at a town rather than at sea" test, which covers a
/// ship still entering the harbour - the town is enterable and its tavern reachable
/// from that moment on - and `+0x39` names that town.
unsafe fn towns_with_player_ships(ships: &ShipsPtr, player_merchant: u16) -> [bool; 0x100] {
    let mut towns = [false; 0x100];
    let mut index = GAME_WORLD_PTR.get_merchant(player_merchant).get_first_ship_index();
    // The chain ends on an out-of-range index; the ship count also caps the walk.
    for _ in 0..ships.get_ships_size() {
        let Some(ship) = ships.get_ship(index) else { break };
        if ship.is_in_port() {
            if let Some(town_index) = ship.get_last_town_index() {
                towns[town_index as usize] = true;
            }
        }
        index = ship.get_next_ship_index_of_merchant();
    }
    towns
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
