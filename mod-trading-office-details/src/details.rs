use std::ffi::CStr;

use p3_api::{
    auto_trader::AutoTraderPtr,
    data::{class48::Class48Ptr, ddraw_set_constant_color, ddraw_set_text_mode, screen_rectangle::Rect, ui_render_text_at},
    facility,
    game_setup,
    game_world::GAME_WORLD_PTR,
    operations::OPERATIONS_PTR,
    ships::ShipsPtr,
    town::{
        construction::{self, ConstructionEta},
        dwellings::CitizenClass,
    },
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
    ddraw_set_text_mode(TEXT_MODE_RIGHT);

    let x = window.get_x();
    let mut y = window.get_y() + FIRST_ROW_Y;
    // Stop before the window's bottom edge instead of drawing past it.
    let last_y = window.get_y() + window.get_height() - ROW_HEIGHT;
    draw_administrator(x, y, window);
    y += ROW_HEIGHT;
    y = draw_pirates(x, y);
    y = draw_winter(x, y, window.get_town_index() as u8);
    y = draw_dwellings(x, y + ROW_HEIGHT, window.get_town_index() as u8);
    draw_constructions(x, y + ROW_HEIGHT, last_y, window.get_town_index() as u8);
}

/// The town's housing per class, as the house info panel's "All dwellings in this town"
/// block shows it: how many houses stand and how full they are. Full houses are the cue to
/// build more; the game shows this only by clicking a town-owned house.
unsafe fn draw_dwellings(x: i32, y: i32, town_index: u8) -> i32 {
    let mut y = y;
    let town = GAME_WORLD_PTR.get_town(town_index);
    font::ddraw_set_font(font::get_header_font());
    draw_text(x + VALUE_X, y, b"Dwellings");
    font::ddraw_set_font(font::get_normal_font());
    y += ROW_HEIGHT;
    for class in CitizenClass::ALL {
        let dwellings = town.get_dwellings(class);
        let Some(percent) = dwellings.occupancy_percent() else {
            continue;
        };
        // Name and count right-aligned in black, the occupancy in its own column fading
        // from black at 90% to red at 95% and above - full houses are the cue to build.
        let line = format!("{}: {}", class.house_name(), dwellings.houses);
        draw_text(x + VALUE_X - OCCUPANCY_COLUMN_WIDTH, y, line.as_bytes());
        ddraw_set_constant_color(occupancy_color(percent));
        ddraw_set_text_mode(TEXT_MODE_LEFT);
        draw_text(x + VALUE_X - OCCUPANCY_COLUMN_WIDTH + OCCUPANCY_GAP, y, format!("{percent}%").as_bytes());
        ddraw_set_text_mode(TEXT_MODE_RIGHT);
        ddraw_set_constant_color(BLACK);
        y += ROW_HEIGHT;
    }
    y
}

/// Width reserved for the occupancy column, and the gap before it.
const OCCUPANCY_COLUMN_WIDTH: i32 = 36;
const OCCUPANCY_GAP: i32 = 6;
const TEXT_MODE_LEFT: u32 = 1;
const TEXT_MODE_RIGHT: u32 = 2;
/// Black up to 90%, red from 95%, a straight blend in between (ARGB, opaque).
fn occupancy_color(percent: i32) -> u32 {
    let red = ((percent - 90).clamp(0, 5) * 255 / 5) as u32;
    0xff00_0000 | (red << 16)
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

/// Everything being built in this town, grouped by owner - the player's own sites, other
/// merchants', the town's - each group in the order the town's workforce pays for them,
/// with the game's own remaining building time (what the building's info panel prints).
/// The game shows sites only on the town map, one panel at a time.
unsafe fn draw_constructions(x: i32, y: i32, last_y: i32, town_index: u8) -> i32 {
    font::ddraw_set_font(font::get_normal_font());
    let mut y = y;
    let town = GAME_WORLD_PTR.get_town(town_index);
    let sites = town.get_pending_construction_sites();
    if sites.is_empty() {
        draw_text(x + VALUE_X, y, b"Nothing under construction");
        return y + ROW_HEIGHT;
    }
    let player_merchant = OPERATIONS_PTR.get_player_merchant_index() as u16;
    let merchants_count = GAME_WORLD_PTR.get_merchants_count();
    let builders = town.get_construction_budget();

    let assigned = town.get_construction_builders_assigned();
    let mut groups: [(&str, Vec<(Vec<u8>, ConstructionEta, u16)>); 3] = [("Yours", Vec::new()), ("Other merchants", Vec::new()), ("Town", Vec::new())];
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
        let name = construction::get_building_name(site.get_building_id()).unwrap_or_else(|| format!("building {:#04x}", site.get_building_id()).into_bytes());
        groups[group].1.push((name, eta, builders_on_it));
    }

    draw_text(x + VALUE_X, y, format!("Under construction ({builders} builders)").as_bytes());
    y += ROW_HEIGHT;
    for (heading, rows) in groups {
        if rows.is_empty() {
            continue;
        }
        if y > last_y {
            break;
        }
        font::ddraw_set_font(font::get_header_font());
        draw_text(x + VALUE_X, y, heading.as_bytes());
        font::ddraw_set_font(font::get_normal_font());
        y += ROW_HEIGHT;
        for (name, eta, builders_on_it) in rows {
            if y > last_y {
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
            let mut line = name;
            line.extend_from_slice(format!(": {when}").as_bytes());
            draw_text(x + VALUE_X, y, &line);
            y += ROW_HEIGHT;
        }
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
