use std::ffi::CStr;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use p3_api::{
    auto_trader::AutoTraderPtr,
    game_world::GAME_WORLD_PTR,
    letters::LettersPtr,
    operations::OPERATIONS_PTR,
    ships::ShipsPtr,
    town::get_town_name_bytes,
    ui::{
        graphics::{GRAPHIC_ID_BONUS, GRAPHIC_ID_CAPTAIN, GRAPHIC_ID_CREW, GRAPHIC_ID_MONEY, GRAPHIC_ID_PIRATE},
        ui_tavern_window::UITavernWindowPtr,
    },
};
use p3_ui::{Align, Cell, Column, Page, Symbol, Table};
use windows::Win32::UI::Input::KeyboardAndMouse::{VK_1, VK_2, VK_3};

pub static CREW: &CStr = c"Crew";
/// The filtered views only cover the towns the player may enter, so their headings say
/// "Known": what they list is what he can see, not what exists.
pub static KNOWN_CREW: &CStr = c"Known crew";
pub static NONE: &CStr = c"none";
pub static MISSIONS: &CStr = c"Missions";
pub static KNOWN_MISSIONS: &CStr = c"Known missions";
pub static OFFER: &CStr = c"Offer";
pub static TERMS: &CStr = c"Terms";
pub static MY_CAPTAINS: &CStr = c"My captains";
/// The key hints, one line on the page's bottom row whatever the view above shows.
pub static HINTS: &CStr = c"1: crew    2: missions    3: my captains";

/// Right-aligned columns name their right edge, so a long town name reaches further left
/// than a short one. `TRADE_X` and `VALUE_X` are shared by the crew and missions views; the
/// captains table has its own tighter columns, laid out from the window's right edge.
const TOWN_X: i32 = 135;
const TRADE_X: i32 = 225;
pub(crate) const VALUE_X: i32 = 345;
/// The kind column is as wide as the widest figure in it, so a narrower one is centred
/// against the others rather than hugging the left.
const KIND_WIDTH: i32 = 26;
/// The crew table: a kind icon marks whose row it is, then the three skills, what he asks,
/// and the town's hireable sailors - captains, pirates and sailors in one page. The game's
/// own icons head the columns: the three skill bonuses out of one sheet, the coin for what
/// he asks, the crew figure for the sailors.
const CREW_COLUMNS: [Column; 7] = [
    Column::right(TOWN_X, 120),
    Column::center(TOWN_X + 6, KIND_WIDTH),
    Column::right(TOWN_X + 70, 30),
    Column::right(TOWN_X + 100, 30),
    Column::right(TOWN_X + 130, 30),
    Column::right(TOWN_X + 180, 50),
    Column::right(TOWN_X + 220, 40),
];
/// The missions table sits further left than the shared columns: its first column only
/// has to hold a town name, and no town name is as wide as the "Known missions" heading
/// over it, so the space it gives up is free. It goes to the terms cell, which is the one
/// that runs out of room.
const MISSION_SHIFT: i32 = 30;
const MISSION_TOWN_X: i32 = TOWN_X - MISSION_SHIFT;
/// The offer column is the one **left**-aligned column on the page. Its values are words of
/// very different length - "Patrol" against "Pirate hunter" - and right-aligning those
/// against the right-aligned town names beside them leaves a ragged gap in the middle of
/// the table. It sits a gutter right of the town column's right edge.
const MISSION_OFFER_X: i32 = MISSION_TOWN_X + 12;
/// Where the terms cell begins: it has to clear the widest offer title, which "Pirate
/// hunter" is, by a gap so the two never touch.
const TERMS_GAP: i32 = 10;
const MISSION_TERMS_LEFT: i32 = TRADE_X - MISSION_SHIFT + TERMS_GAP;
/// The terms hang off the window's right edge, inside the frame; a narrow window falls back
/// to the column position the other views share.
const TERMS_RIGHT_MARGIN: i32 = 15;
pub(crate) const FIRST_ROW_Y: i32 = 12;

/// What the page shows, switched by a key press and kept across openings of the
/// window: 1 the captains and pirates, 2 the mission offers, with alt selecting
/// every town instead of only the enterable ones; 3 the player's own captains, with
/// alt revealing their skill caps. The keys go through the shared hotkey registry,
/// registered only while this page (the tavern's -1 page) is on screen - see ffi.rs
/// for the scope events.
static VIEW: AtomicU8 = AtomicU8::new(VIEW_CREW);
static SHOW_ALL_TOWNS: AtomicBool = AtomicBool::new(false);

const VIEW_CREW: u8 = 0;
const VIEW_MISSIONS: u8 = 1;
const VIEW_CAPTAINS: u8 = 2;

/// Whether key 3's view is the one selected - the scrollbar rides along only then.
pub(crate) fn captains_view_selected() -> bool {
    VIEW.load(Ordering::Relaxed) == VIEW_CAPTAINS
}

pub(crate) const PAGE_KEY_CREW: u32 = VK_1.0 as u32;
pub(crate) const PAGE_KEY_MISSIONS: u32 = VK_2.0 as u32;
pub(crate) const PAGE_KEY_CAPTAINS: u32 = VK_3.0 as u32;

/// The page's key handler: registered while the details page is on screen, so no
/// screen check is needed here. Declines (returns 0), as the polling before it
/// effectively did - the game sees the keys too.
#[no_mangle]
pub(crate) unsafe extern "C" fn page_hotkeys(vk: u32, mods: u32) -> u32 {
    let view = match vk {
        PAGE_KEY_MISSIONS => VIEW_MISSIONS,
        PAGE_KEY_CAPTAINS => VIEW_CAPTAINS,
        _ => VIEW_CREW,
    };
    VIEW.store(view, Ordering::Relaxed);
    SHOW_ALL_TOWNS.store(mods & p3_api::hotkeys::MOD_ALT != 0, Ordering::Relaxed);
    0
}

/// The page, with its bottom row kept for the key hints.
pub(crate) fn page(window: &UITavernWindowPtr) -> Page {
    Page::new(window, FIRST_ROW_Y).reserve_bottom_rows(1)
}

/// Submit the window's area to the renderer, from the window's update method
/// (`0x005CD540`) - the phase the game itself uses for this call.
pub(crate) unsafe fn invalidate(window: UITavernWindowPtr) {
    page(&window).invalidate();
}

/// No window title: the page the game itself draws here has none, and
/// `render_window_title` would spend the top of the window on a banner graphic that
/// the tables need for rows.
pub(crate) unsafe fn draw_page(window: UITavernWindowPtr) {
    let page = page(&window);
    page.reset_state();

    let all_towns = SHOW_ALL_TOWNS.load(Ordering::Relaxed);
    let view = VIEW.load(Ordering::Relaxed);
    let towns = enterable_towns(all_towns);

    match view {
        VIEW_MISSIONS => {
            let heading = if all_towns { MISSIONS } else { KNOWN_MISSIONS };
            draw_missions(&page, heading, &towns, all_towns);
        }
        VIEW_CAPTAINS => {
            crate::my_captains::draw(&page, MY_CAPTAINS, all_towns);
        }
        _ => {
            let heading = if all_towns { CREW } else { KNOWN_CREW };
            draw_crew(&page, heading, &towns, all_towns);
        }
    }

    page.reset_state();
    page.draw_text(page.abs_x(VALUE_X), page.bottom_row_y(), Align::Right, HINTS.to_bytes());
}

/// One row per mission a tavern's side room offers, by town.
///
/// An offer is a letter in the player's own mailbox: the side room walks his letter
/// chain with the game's predicate at `0x004D7900` - type `0x71`, the town byte matching
/// the tavern, and a descriptor date still in the future - and titles its page with the
/// letter's own title. What the offer is worth goes in one cell: where the cargo goes, how
/// much of it, and the sum, with the game's own load and coin symbols.
unsafe fn draw_missions(page: &Page, heading: &CStr, towns: &[u8], all_towns: bool) -> i32 {
    let terms_right = (page.width - TERMS_RIGHT_MARGIN).max(VALUE_X);
    let table = Table::new(
        page,
        [
            Column::right(MISSION_TOWN_X, 100),
            Column::left(MISSION_OFFER_X, MISSION_TERMS_LEFT - MISSION_OFFER_X),
            // A cell narrower than its text would wrap onto the row below.
            Column::right(terms_right, terms_right - MISSION_TERMS_LEFT),
        ],
    );
    let mut y = table.header(page.top, &[heading.into(), OFFER.into(), TERMS.into()], None);

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
            if !table.fits(y) {
                return y;
            }
            found = true;
            let town = get_town_name_bytes(*town_index).map(Cell::from).unwrap_or(Cell::Empty);
            let title = letter.get_title_bytes().map(Cell::from).unwrap_or(Cell::Empty);
            // A smuggler names his town only once the order is accepted, so the filtered
            // view - what the player could know - leaves that part out.
            let mut terms: Vec<Vec<u8>> = Vec::new();
            let disclosed = all_towns || !letter.tavern_mission_conceals_destination();
            if let Some(destination) = letter.get_destination_town_index().filter(|_| disclosed) {
                if let Some(town) = get_town_name_bytes(destination) {
                    terms.push(town);
                }
            }
            if let Some(loads) = letter.get_required_loads() {
                terms.push(format!("{loads}{}", Symbol::Load.escape()).into_bytes());
            }
            if let Some(reward) = letter.get_reward() {
                terms.push(format!("{reward}{}", Symbol::Coin.escape()).into_bytes());
            }
            // A treasure map is the one offer that costs money instead of paying it, so
            // its sum goes in with a minus.
            if let Some(price) = letter.get_asking_price() {
                terms.push(format!("-{price}{}", Symbol::Coin.escape()).into_bytes());
            }
            let terms = if terms.is_empty() { Cell::Empty } else { Cell::rich(terms.join(&b", "[..])) };
            y = table.row(y, &[town, title, terms]);
            index = letter.get_next_index();
        }
    }
    if !found {
        y = table.row(y, &[NONE.into(), Cell::Empty, Cell::Empty]);
    }
    y
}

/// The whole hiring picture of a town in one table: a row per hireable captain or pirate,
/// marked by the game's own figure for which he is, with his three skills and what he
/// asks - a daily wage for a captain, a share of the loot for a pirate - and the town's
/// hireable sailors on its first row. A town with nobody waiting still gets a row, so its
/// sailors are visible; anyone waiting there follows on that row and the ones under it.
///
/// A pirate's skills are left blank unless `all_towns`: the filtered table shows what the
/// player could know, and only the unrestricted one gives them away.
unsafe fn draw_crew(page: &Page, heading: &CStr, towns: &[u8], all_towns: bool) -> i32 {
    let table = Table::new(page, CREW_COLUMNS);
    let mut y = table.header(
        page.top,
        &[
            heading.into(),
            Cell::Empty,
            Cell::graphic_frame(GRAPHIC_ID_BONUS, 0),
            Cell::graphic_frame(GRAPHIC_ID_BONUS, 1),
            Cell::graphic_frame(GRAPHIC_ID_BONUS, 2),
            Cell::graphic(GRAPHIC_ID_MONEY),
            Cell::graphic(GRAPHIC_ID_CREW),
        ],
        None,
    );

    let ships = ShipsPtr::new();
    let merchant = GAME_WORLD_PTR.get_merchant(OPERATIONS_PTR.get_player_merchant_index() as u16);

    for town_index in towns {
        if !table.fits(y) {
            break;
        }
        let mut town = get_town_name_bytes(*town_index).map(Cell::from).unwrap_or(Cell::Empty);
        let mut sailors = Cell::number(merchant.get_available_sailors(*town_index) as i32);

        let waiting = hireable_auto_traders(&ships, *town_index);
        if waiting.is_empty() {
            // Nobody waiting, so the town's own row is all it gets.
            y = table.row(y, &[town, Cell::Empty, Cell::Empty, Cell::Empty, Cell::Empty, Cell::Empty, sailors]);
            continue;
        }
        for (is_pirate, trader) in waiting {
            if !table.fits(y) {
                break;
            }
            let kind = Cell::graphic(if is_pirate { GRAPHIC_ID_PIRATE } else { GRAPHIC_ID_CAPTAIN });
            // The icon order is the game's own captain panel's: trade under the first
            // (0x005CFA55 reads +0xA), navigation under the second (0x005CFB16).
            let [trade, navigation, combat] = if all_towns || !is_pirate {
                [trader.get_trade_skill(), trader.get_navigation_skill(), trader.get_combat_skill()]
                    .map(|skill| Cell::number(AutoTraderPtr::skill_level(skill) as i32))
            } else {
                [Cell::Empty, Cell::Empty, Cell::Empty]
            };
            let pay = if is_pirate {
                Cell::number_with(trader.get_pirate_loot_share_percent() as i32, " %")
            } else {
                Cell::number(trader.get_daily_wage() as i32)
            };
            // The town's name and sailors go on the first of its rows only.
            let town_cell = std::mem::replace(&mut town, Cell::Empty);
            let sailors_cell = std::mem::replace(&mut sailors, Cell::Empty);
            y = table.row(y, &[town_cell, kind, trade, navigation, combat, pay, sailors_cell]);
        }
    }
    y
}

/// The towns the player may legally enter, which is where he can hire: the scrollmap's own
/// double-click test ([p3_api::game_world::GameWorldPtr::can_merchant_enter_town]) - a ship
/// of his in the town, or a trading office. With `all_towns`, every town instead,
/// enterable or not.
unsafe fn enterable_towns(all_towns: bool) -> Vec<u8> {
    let player_merchant = OPERATIONS_PTR.get_player_merchant_index() as u16;
    (0..GAME_WORLD_PTR.get_towns_count().min(0xff) as u8)
        .filter(|&town_index| all_towns || GAME_WORLD_PTR.can_merchant_enter_town(player_merchant, town_index))
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
