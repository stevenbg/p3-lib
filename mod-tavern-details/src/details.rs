use std::ffi::CStr;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use p3_api::{
    auto_trader::AutoTraderPtr,
    data::{class48::Class48Ptr, ddraw_set_constant_color, ddraw_set_text_mode, screen_rectangle::Rect, ui_render_text_at},
    game_world::GAME_WORLD_PTR,
    letters::LettersPtr,
    operations::OPERATIONS_PTR,
    ships::ShipsPtr,
    town::get_town_name_bytes,
    ui::{
        font,
        graphics::{
            draw_graphic, draw_graphic_frame, graphic_frame_size, GRAPHIC_ID_BONUS, GRAPHIC_ID_CAPTAIN, GRAPHIC_ID_CREW, GRAPHIC_ID_MONEY,
            GRAPHIC_ID_PIRATE,
        },
        rect_clipper_stuff,
        rich_text::{draw_rich_text, TAVERN_WINDOW_LAYOUT_OFFSET},
        ui_tavern_window::UITavernWindowPtr,
    },
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VIRTUAL_KEY, VK_1, VK_2, VK_MENU};

pub static CREW: &CStr = c"Crew";
/// The filtered views only cover the towns the player may enter, so their headings say
/// "Known": what they list is what he can see, not what exists.
pub static KNOWN_CREW: &CStr = c"Known crew";
pub static NONE: &CStr = c"none";
pub static MISSIONS: &CStr = c"Missions";
pub static KNOWN_MISSIONS: &CStr = c"Known missions";
pub static OFFER: &CStr = c"Offer";
pub static TERMS: &CStr = c"Terms";
pub static CREW_HINT: &CStr = c"1: crew";
pub static MISSIONS_HINT: &CStr = c"2: missions";

/// Text mode 2 draws right-aligned: every column x below is the right edge of that
/// column, so a long town name reaches further left than a short one. `TRADE_X` and
/// `VALUE_X` are shared by the sailors and missions views; the captains table has its own
/// tighter columns below.
const TOWN_X: i32 = 135;
const TRADE_X: i32 = 225;
const VALUE_X: i32 = 345;
/// Icons are 16x16, and heading a column with one instead of a word lets the skill columns
/// of the captains table sit closer together than the shared positions above.
const ICON: i32 = 16;
/// The kind column is as wide as the widest figure in it, so a narrower one can be centred
/// against the others rather than hugging the left.
const KIND_WIDTH: i32 = 26;
/// The crew table: a kind icon marks whose row it is, then the three skills, what he asks,
/// and the town's hireable sailors - captains, pirates and sailors in one page.
const KIND_X: i32 = TOWN_X + 6;
const SKILL_1_X: i32 = TOWN_X + 70;
const SKILL_2_X: i32 = TOWN_X + 100;
const SKILL_3_X: i32 = TOWN_X + 130;
const PAY_X: i32 = TOWN_X + 180;
const CREW_X: i32 = TOWN_X + 220;
const FIRST_ROW_Y: i32 = 12;
const ROW_HEIGHT: i32 = 16;
const BLACK: u32 = 0xff000000;

/// What the page shows, switched by a key press rather than held, and kept across
/// openings of the window: 1 the captains and pirates, 2 the sailor pools, 3 the mission
/// offers, with alt selecting every town instead of only the enterable ones. Number keys
/// rather than function keys, because mod-auto-supply's F3 builds a trade route from
/// anywhere and its keyboard hook cannot see which page is on screen.
static VIEW: AtomicU8 = AtomicU8::new(VIEW_CREW);
static SHOW_ALL_TOWNS: AtomicBool = AtomicBool::new(false);
static KEY_1_WAS_DOWN: AtomicBool = AtomicBool::new(false);
static KEY_2_WAS_DOWN: AtomicBool = AtomicBool::new(false);

const VIEW_CREW: u8 = 0;
const VIEW_MISSIONS: u8 = 1;

/// Read the page's keys, called once per frame from the update phase and only while the
/// page is actually on screen, so nothing outside this page is affected and no global
/// keyboard hook is involved. Acts on the down edge, so a key switches the view rather
/// than needing to be held.
pub(crate) fn poll_keys() {
    for (key, was_down, view) in [
        (VK_1, &KEY_1_WAS_DOWN, VIEW_CREW),
        (VK_2, &KEY_2_WAS_DOWN, VIEW_MISSIONS),
    ] {
        if key_pressed(key, was_down) {
            VIEW.store(view, Ordering::Relaxed);
            SHOW_ALL_TOWNS.store(key_down(VK_MENU), Ordering::Relaxed);
        }
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
    let view = VIEW.load(Ordering::Relaxed);
    let towns = enterable_towns(all_towns);

    match view {
        VIEW_MISSIONS => {
            let heading = if all_towns { MISSIONS } else { KNOWN_MISSIONS };
            y = draw_missions(window, y, last_y, heading, &towns, all_towns);
        }
        _ => {
            let heading = if all_towns { CREW } else { KNOWN_CREW };
            y = draw_crew(x, y, last_y, heading, &towns, all_towns);
        }
    }

    if y <= last_y {
        font::ddraw_set_font(font::get_normal_font());
        let other = if view == VIEW_MISSIONS { CREW_HINT } else { MISSIONS_HINT };
        draw_text(x + VALUE_X, y + ROW_HEIGHT, other.to_bytes());
    }
}

/// One row per mission a tavern's side room offers, by town.
///
/// An offer is a letter in the player's own mailbox: the side room walks his letter
/// chain with the game's predicate at `0x004D7900` - type `0x71`, the town byte matching
/// the tavern, and a descriptor date still in the future - and titles its page with the
/// start of the letter's text, which is where "Patrol" or "Escort" comes from.
/// The width of the terms cell, wide enough for a town, a cargo and a sum; the rich-text
/// pass wraps at this, so too narrow a cell would spill onto a second line.
const TERMS_WIDTH: i32 = 170;

unsafe fn draw_missions(window: UITavernWindowPtr, y: i32, last_y: i32, heading: &CStr, towns: &[u8], all_towns: bool) -> i32 {
    let x = window.get_x();
    // The terms hang off the window's right edge; a narrow window falls back to the
    // column position the other views share.
    let terms_right = (window.get_width() - 15).max(VALUE_X);

    let mut y = y;
    font::ddraw_set_font(font::get_header_font());
    draw_text(x + TOWN_X, y, heading.to_bytes());
    draw_text(x + TRADE_X, y, OFFER.to_bytes());
    draw_text(x + terms_right, y, TERMS.to_bytes());
    y += ROW_HEIGHT;

    font::ddraw_set_font(font::get_normal_font());
    let letters = LettersPtr::new();
    let player_merchant = OPERATIONS_PTR.get_player_merchant_index() as u16;
    let merchant = GAME_WORLD_PTR.get_merchant(player_merchant);
    let mut found = false;
    for town_index in towns {
        let mut index = merchant.get_first_letter_index();
        // Each hit continues the walk from that letter's own next link, the way the side
        // room does; the pool size caps it against a cycle.
        for _ in 0..letters.get_size() {
            let Some(found_index) = letters.find_tavern_mission(index, *town_index as u16) else { break };
            let Some(letter) = letters.get_letter(found_index) else { break };
            // Skip what the side room skips: an offer a rival merchant has taken.
            if !letter.tavern_mission_is_open(player_merchant) {
                index = letter.get_next_index();
                continue;
            }
            if y > last_y {
                return y;
            }
            found = true;
            if let Some(town) = get_town_name_bytes(*town_index) {
                draw_text(x + TOWN_X, y, &town);
            }
            if let Some(title) = letter.get_title_bytes() {
                draw_text(x + TRADE_X, y, &title);
            }
            // What the offer is worth, in one cell: where the cargo goes, how much of it,
            // and the sum. A smuggler names his town only once the order is accepted, so
            // the filtered view - what the player could know - leaves that part out.
            let mut terms: Vec<Vec<u8>> = Vec::new();
            let disclosed = all_towns || !letter.tavern_mission_conceals_destination();
            if let Some(destination) = letter.get_destination_town_index().filter(|_| disclosed) {
                if let Some(town) = get_town_name_bytes(destination) {
                    terms.push(town);
                }
            }
            if let Some(loads) = letter.get_required_loads() {
                terms.push(format!("{loads}\\L").into_bytes());
            }
            if let Some(reward) = letter.get_reward() {
                terms.push(format!("{reward}\\C").into_bytes());
            }
            // A treasure map is the one offer that costs money instead of paying it, so
            // its sum goes in with a minus.
            if let Some(price) = letter.get_asking_price() {
                terms.push(format!("-{price}\\C").into_bytes());
            }
            if !terms.is_empty() {
                // Through the framework's rich-text pass, for the game's own cargo and
                // coin symbols. `\r` offsets the line by minus its own width
                // (`0x00420AB2`), so the x argument is the cell's RIGHT edge, while the
                // width argument only bounds word wrap. That pass sets its own font and
                // colour, so the page's state is restored after it.
                let mut cell = b"\\r".to_vec();
                cell.extend(terms.join(&b", "[..]));
                cell.push(0);
                draw_rich_text(
                    window.address + TAVERN_WINDOW_LAYOUT_OFFSET,
                    &cell,
                    x + terms_right,
                    y,
                    TERMS_WIDTH,
                    ROW_HEIGHT,
                    BLACK,
                );
                ddraw_set_constant_color(BLACK);
                ddraw_set_text_mode(2);
                font::ddraw_set_font(font::get_normal_font());
            }
            y += ROW_HEIGHT;
            index = letter.get_next_index();
        }
    }
    if !found {
        draw_text(x + TOWN_X, y, NONE.to_bytes());
        y += ROW_HEIGHT;
    }
    y
}

/// A graphic's width, for placing it against right-aligned text; `ICON` covers the case
/// where the graphic is missing.
unsafe fn icon_width(id: u32) -> i32 {
    graphic_frame_size(id, 0).map(|(width, _)| width).unwrap_or(ICON)
}

fn key_down(key: VIRTUAL_KEY) -> bool {
    (unsafe { GetKeyState(key.0 as i32) } as u16) & 0x8000 != 0
}

/// The whole hiring picture of a town in one table: a row per hireable captain or pirate,
/// marked by the game's own figure for which he is, with his three skills and what he
/// asks - a daily wage for a captain, a share of the loot for a pirate - and the town's
/// hireable sailors on its first row. A town with nobody waiting still gets a row, so its
/// sailors are visible.
///
/// A pirate's skills are left blank unless `all_towns`: the filtered table shows what the
/// player could know, and only the unrestricted one gives them away.
unsafe fn draw_crew(x: i32, y: i32, last_y: i32, heading: &CStr, towns: &[u8], all_towns: bool) -> i32 {
    let mut y = y;
    font::ddraw_set_font(font::get_header_font());
    draw_text(x + TOWN_X, y, heading.to_bytes());
    // The game's own icons head the columns: the three skill bonuses out of one sheet, the
    // coin for what he asks, the crew figure for the sailors. Columns are right-aligned, so
    // an icon heading one sits its own width to the left.
    for (frame, column) in [(0, SKILL_1_X), (1, SKILL_2_X), (2, SKILL_3_X)] {
        draw_graphic_frame(GRAPHIC_ID_BONUS, frame, x + column - icon_width(GRAPHIC_ID_BONUS), y);
    }
    // The icons are not one size, so each is placed by its own width to line its right edge
    // up with the numbers under it.
    draw_graphic(GRAPHIC_ID_MONEY, x + PAY_X - icon_width(GRAPHIC_ID_MONEY), y);
    draw_graphic(GRAPHIC_ID_CREW, x + CREW_X - icon_width(GRAPHIC_ID_CREW), y);
    // The blits leave the constant colour white; everything below is text again.
    ddraw_set_constant_color(BLACK);
    y += ROW_HEIGHT;

    font::ddraw_set_font(font::get_normal_font());
    let ships = ShipsPtr::new();
    let merchant = GAME_WORLD_PTR.get_merchant(OPERATIONS_PTR.get_player_merchant_index() as u16);

    for town_index in towns {
        if y > last_y {
            break;
        }
        // The town's own row carries its name and its sailors; anyone waiting there follows
        // on this row and the ones under it.
        if let Some(town) = get_town_name_bytes(*town_index) {
            draw_text(x + TOWN_X, y, &town);
        }
        draw_number(x + CREW_X, y, merchant.get_available_sailors(*town_index) as i32, "");

        let waiting = hireable_auto_traders(&ships, *town_index);
        if waiting.is_empty() {
            // Nobody waiting, so the town's own row is all it gets.
            y += ROW_HEIGHT;
            continue;
        }
        for (is_pirate, trader) in waiting {
            if y > last_y {
                break;
            }
            let kind = if is_pirate { GRAPHIC_ID_PIRATE } else { GRAPHIC_ID_CAPTAIN };
            draw_graphic(kind, x + KIND_X + (KIND_WIDTH - icon_width(kind)) / 2, y);
            ddraw_set_constant_color(BLACK);
            if all_towns || !is_pirate {
                draw_number(x + SKILL_1_X, y, AutoTraderPtr::skill_level(trader.get_navigation_skill()) as i32, "");
                draw_number(x + SKILL_2_X, y, AutoTraderPtr::skill_level(trader.get_trade_skill()) as i32, "");
                draw_number(x + SKILL_3_X, y, AutoTraderPtr::skill_level(trader.get_combat_skill()) as i32, "");
            }
            if is_pirate {
                draw_number(x + PAY_X, y, trader.get_pirate_loot_share_percent() as i32, " %");
            } else {
                draw_number(x + PAY_X, y, trader.get_daily_wage() as i32, "");
            }
            y += ROW_HEIGHT;
        }
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

/// The captains and pirates nobody employs in one town, with `true` marking a pirate. The
/// records chained to a town are the ones sitting in its tavern, and an unemployed one
/// (merchant `0xFF`) is the one its resolver hands out.
unsafe fn hireable_auto_traders(ships: &ShipsPtr, town_index: u8) -> Vec<(bool, AutoTraderPtr)> {
    let mut waiting = Vec::new();
    let mut index = GAME_WORLD_PTR.get_town(town_index).get_auto_trader_chain_head();
    // The chain ends on an out-of-range index; the count also caps the walk.
    for _ in 0..ships.get_auto_traders_size() {
        let Some(trader) = ships.get_auto_trader(index) else { break };
        if trader.get_merchant_index() == 0xff {
            waiting.push((trader.is_pirate(), trader));
        }
        index = trader.get_next_index();
    }
    waiting
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
