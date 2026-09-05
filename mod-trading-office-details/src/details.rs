use std::ffi::CStr;

use p3_api::{
    auto_trader::AutoTraderPtr,
    facility, game_setup,
    game_world::GAME_WORLD_PTR,
    operations::OPERATIONS_PTR,
    ships::ShipsPtr,
    town::{
        construction::{self, ConstructionEta},
        dwellings::CitizenClass,
    },
    ui::ui_trading_office_window::UITradingOfficeWindowPtr,
};
use p3_ui::{Align, Cell, Column, Page, Table};

pub static NO_ADMINISTRATOR: &CStr = c"No administrator employed";

/// The page is one right-aligned column of lines ending at this edge.
const VALUE_X: i32 = 345;
const FIRST_ROW_Y: i32 = 12;
/// The dwellings table: the class and its house count end left of the occupancy column,
/// which starts a gutter later so the percentages line up.
const OCCUPANCY_COLUMN_WIDTH: i32 = 36;
const OCCUPANCY_GAP: i32 = 6;
const DWELLING: Column = Column::right(VALUE_X - OCCUPANCY_COLUMN_WIDTH, 200);
const OCCUPANCY: Column = Column::left(VALUE_X - OCCUPANCY_COLUMN_WIDTH + OCCUPANCY_GAP, OCCUPANCY_COLUMN_WIDTH);

fn page(window: &UITradingOfficeWindowPtr) -> Page {
    Page::new(window, FIRST_ROW_Y)
}

pub(crate) unsafe fn invalidate(window: UITradingOfficeWindowPtr) {
    page(&window).invalidate();
}

/// No window title: the page the game itself draws here has none, and
/// `render_window_title` would spend the top of the window on a banner graphic.
pub(crate) unsafe fn draw_page(window: UITradingOfficeWindowPtr) {
    let page = page(&window);
    page.reset_state();
    let town_index = window.get_town_index() as u8;

    let mut y = page.top;
    y = draw_administrator(&page, y, window);
    y = draw_pirates(&page, y);
    y = draw_winter(&page, y, town_index);
    y = draw_dwellings(&page, y + page.row_height, town_index);
    draw_constructions(&page, y + page.row_height, town_index);
}

/// One right-aligned line at the page's value edge.
unsafe fn line(page: &Page, y: i32, text: impl Into<Vec<u8>>) -> i32 {
    page.line(VALUE_X, y, Align::Right, &Cell::owned(text.into()))
}

/// The town's housing per class, as the house info panel's "All dwellings in this town"
/// block shows it: how many houses stand and how full they are. Full houses are the cue to
/// build more; the game shows this only by clicking a town-owned house. A `*` in front of
/// a class means a house of that type is already on the town's construction list.
unsafe fn draw_dwellings(page: &Page, y: i32, town_index: u8) -> i32 {
    let mut y = y;
    let town = GAME_WORLD_PTR.get_town(town_index);
    let being_built: Vec<CitizenClass> = town
        .get_pending_construction_sites()
        .iter()
        .filter_map(|(_, site)| CitizenClass::from_dwelling_building_id(site.get_building_id()))
        .collect();
    y = page.heading(VALUE_X, y, Align::Right, b"Dwellings");
    let table = Table::new(page, [DWELLING, OCCUPANCY]);
    for class in CitizenClass::ALL {
        let dwellings = town.get_dwellings(class);
        let Some(percent) = dwellings.occupancy_percent() else {
            continue;
        };
        // Name and count in black, the occupancy in its own column fading from black at 90%
        // to red at 95% and above - full houses are the cue to build.
        let mark = if being_built.contains(&class) { "* " } else { "" };
        let line = format!("{mark}{}: {}", class.house_name(), dwellings.houses);
        let occupancy = Cell::from(format!("{percent}%")).colored(occupancy_color(percent));
        y = table.row(y, &[line.into(), occupancy]);
    }
    y
}

/// Black up to 90%, red from 95%, a straight blend in between (ARGB, opaque).
fn occupancy_color(percent: i32) -> u32 {
    let red = ((percent - 90).clamp(0, 5) * 255 / 5) as u32;
    0xff00_0000 | (red << 16)
}

/// The two pirate facts the game never states, both driven by the single "Pirates activity"
/// setting rather than by the difficulty preset (which merely writes that setting along with
/// every other one).
unsafe fn draw_pirates(page: &Page, y: i32) -> i32 {
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
        y = line(page, y, format!("Pirate bands roaming: {bands} (activity {level})"));
    }

    // A free pirate robs a merchant once `rank_in_home_town + activity >= 2`, so the higher
    // the setting the lower the rank it settles for. Showing the player's own rank next to
    // the threshold makes it clear whether he is already fair game.
    if let Some(threshold) = game_setup::pirate_attack_rank_threshold() {
        let merchant = GAME_WORLD_PTR.get_merchant(OPERATIONS_PTR.get_player_merchant_index() as u16);
        let rank = merchant.get_rank_in(merchant.get_hometown_index());
        let verdict = if rank >= threshold { "you qualify" } else { "you are beneath notice" };
        y = line(page, y, format!("Pirates rob from home rank {threshold} (yours {rank}: {verdict})"));
    }
    y
}

/// Which months carry the crop penalty, whether one of them is running, and what it costs.
/// The game shows none of this, and the size of it is worth knowing before planning a
/// farming town's supply.
unsafe fn draw_winter(page: &Page, y: i32, town_index: u8) -> i32 {
    let mut y = y;

    // The month list is `GameWorldPtr::WINTER_MONTHS` (11, 0, 1 - the field is zero-based).
    let now = if GAME_WORLD_PTR.is_winter() { " - in force now" } else { "" };
    y = line(page, y, format!("Crop winter: Dec, Jan, Feb{now}"));

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
        y = line(page, y, format!("Winter output: {}", parts.join(", ")));
    }
    y
}

/// Everything being built in this town, grouped by owner - the player's own sites, other
/// merchants', the town's - each group in the order the town's workforce pays for them,
/// with the game's own remaining building time (what the building's info panel prints).
/// The game shows sites only on the town map, one panel at a time.
unsafe fn draw_constructions(page: &Page, y: i32, town_index: u8) -> i32 {
    let mut y = y;
    let town = GAME_WORLD_PTR.get_town(town_index);
    let sites = town.get_pending_construction_sites();
    if sites.is_empty() {
        return line(page, y, "Nothing under construction");
    }
    let player_merchant = OPERATIONS_PTR.get_player_merchant_index() as u16;
    let merchants_count = GAME_WORLD_PTR.get_merchants_count();
    let builders = town.get_construction_budget();

    let assigned = town.get_construction_builders_assigned();
    let mut groups: [(&str, Vec<(Vec<u8>, ConstructionEta, u16)>); 3] =
        [("Yours", Vec::new()), ("Other merchants", Vec::new()), ("Town", Vec::new())];
    for ((index, site), builders_on_it) in sites.into_iter().zip(assigned) {
        let eta = town.get_construction_days_remaining(index);
        let owner = site.get_owner_merchant_index() as u16;
        let group = if owner >= merchants_count {
            2
        } else if owner == player_merchant {
            0
        } else {
            1
        };
        let name = construction::get_building_name(site.get_building_id())
            .unwrap_or_else(|| format!("building {:#04x}", site.get_building_id()).into_bytes());
        groups[group].1.push((name, eta, builders_on_it));
    }

    y = line(page, y, format!("Under construction ({builders} builders)"));
    for (heading, rows) in groups {
        if rows.is_empty() {
            continue;
        }
        if !page.fits(y) {
            break;
        }
        y = page.heading(VALUE_X, y, Align::Right, heading.as_bytes());
        for (name, eta, builders_on_it) in rows {
            if !page.fits(y) {
                break;
            }
            let when = match eta {
                ConstructionEta::Days(1) => format!("1 day, {builders_on_it} builders"),
                ConstructionEta::Days(days) => format!("{days} days, {builders_on_it} builders"),
                ConstructionEta::Finishing => "finishing".to_string(),
                ConstructionEta::StartsSoon => "waiting, starts soon".to_string(),
                ConstructionEta::Queued { position } => format!("waiting, position {position}"),
                ConstructionEta::Invalid => "?".to_string(),
            };
            let mut text = name;
            text.extend_from_slice(format!(": {when}").as_bytes());
            y = line(page, y, text);
        }
    }
    y
}

/// This office's administrator pays 2% less per trade skill level on everything he
/// buys - an effect the game shows nowhere.
unsafe fn draw_administrator(page: &Page, y: i32, window: UITradingOfficeWindowPtr) -> i32 {
    let town_index = window.get_town_index() as u8;
    let player_merchant = OPERATIONS_PTR.get_player_merchant_index();
    let Some(office) = GAME_WORLD_PTR.get_office_in_of(town_index, player_merchant as _) else {
        return y + page.row_height;
    };

    match ShipsPtr::new().get_auto_trader(office.get_administrator_index()) {
        None => line(page, y, NO_ADMINISTRATOR.to_bytes()),
        Some(administrator) => {
            let level = AutoTraderPtr::skill_level(administrator.get_trade_skill());
            let discount = 100 - administrator.get_buy_percent_paid();
            line(page, y, format!("Administrator's buying discount: {discount}%, level {level}"))
        }
    }
}
