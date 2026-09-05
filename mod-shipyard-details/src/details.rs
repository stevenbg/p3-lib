//! The shipyard details page: the yard's figures the game shows nowhere, and where each
//! ship type's quality stands.

use std::ffi::CStr;

use p3_api::{
    data::{fill_p3_string, render_window_title, statics::get_shipyard_level_requirements},
    game_world::GAME_WORLD_PTR,
    ui::ui_shipyard_window::UIShipyardWindowPtr,
};
use p3_page::{Cell, Column, Page, Table};

/// The page starts below the title banner and the window's own art.
const FIRST_ROW_Y: i32 = 200;
/// The window's own row pitch.
const ROW_HEIGHT: i32 = 20;
/// Experience is stored scaled by this; the page shows it in levels.
const EXPERIENCE_SCALE: f32 = 2800.0;
/// The highest quality level a ship type reaches.
const MAX_QUALITY_LEVEL: i8 = 3;

/// The yard's figures: a label ending at the label edge, its value ending right of it.
const FACTS: [Column; 2] = [Column::right(200, 180), Column::right(300, 90)];
/// The per-ship-type table: a label, then one column per type.
const SHIPS: [Column; 5] = [
    Column::right(100, 90),
    Column::right(160, 60),
    Column::right(220, 60),
    Column::right(280, 60),
    Column::right(340, 60),
];

pub static TITLE: &CStr = c"Details";
pub static EMPLOYEES: &CStr = c"Employees";
pub static EXPERIENCE: &CStr = c"Experience (scaled)";
pub static PENDING_EXPERIENCE: &CStr = c"Pending Experience (scaled)";
pub static UTILIZATION_MARKUP: &CStr = c"Utilization Markup";
pub static SNAIKKA: &CStr = c"Snaikka";
pub static CRAYER: &CStr = c"Crayer";
pub static COG: &CStr = c"Cog";
pub static HULK: &CStr = c"Hulk";
pub static QUALITY_LEVEL: &CStr = c"Quality Level";
pub static REQUIRED_XP: &CStr = c"Required XP";

/// The drawing state every details page uses, set on open and again before each draw.
pub(crate) unsafe fn prepare_drawing_state() {
    p3_page::page::prepare_drawing_state();
}

fn page(window: &UIShipyardWindowPtr) -> Page {
    let mut page = Page::new(window, FIRST_ROW_Y);
    page.row_height = ROW_HEIGHT;
    page
}

pub(crate) unsafe fn invalidate(window: UIShipyardWindowPtr) {
    page(&window).invalidate();
}

pub(crate) unsafe fn draw_page(window: UIShipyardWindowPtr) {
    let town = GAME_WORLD_PTR.get_town(window.get_town_index() as _);
    let shipyard_facility = town.get_facility(1);
    let shipyard = town.get_shipyard();
    let levels = shipyard.get_current_quality_levels();
    let requirements = get_shipyard_level_requirements();

    prepare_drawing_state();
    let mut title_p3_string: u32 = 0;
    fill_p3_string((&mut title_p3_string) as *mut _ as _, TITLE.to_bytes());
    render_window_title(title_p3_string as _, window.address as _);

    let page = page(&window);
    page.reset_state();
    let facts = Table::new(&page, FACTS);
    let mut y = page.top;
    y = facts.row(y, &[EMPLOYEES.into(), (shipyard_facility.get_employees() as i32).into()]);
    y = facts.row(y, &[UTILIZATION_MARKUP.into(), format!("{:.2}", shipyard.get_utilization_markup()).into()]);
    y = facts.row(
        y,
        &[
            PENDING_EXPERIENCE.into(),
            format!("{:.2}", shipyard.get_pending_experience() as f32 / EXPERIENCE_SCALE).into(),
        ],
    );
    y = facts.row(y, &[EXPERIENCE.into(), format!("{:.2}", shipyard.get_experience() as f32 / EXPERIENCE_SCALE).into()]);

    let ships = Table::new(&page, SHIPS);
    y = ships.header(y, &[Cell::Empty, SNAIKKA.into(), CRAYER.into(), COG.into(), HULK.into()], None);
    let current = [levels.snaikka_level, levels.crayer_level, levels.cog_level, levels.hulk_level];
    y = ships.row(
        y,
        &[
            QUALITY_LEVEL.into(),
            (current[0] as i32).into(),
            (current[1] as i32).into(),
            (current[2] as i32).into(),
            (current[3] as i32).into(),
        ],
    );
    // What the next level costs; a type at the top has nothing left to show.
    let next = |level: i8, table: &[u16; 4]| -> Cell<'static> {
        if (0..MAX_QUALITY_LEVEL).contains(&level) {
            Cell::number(table[level as usize + 1] as i32)
        } else {
            Cell::Empty
        }
    };
    ships.row(
        y,
        &[
            REQUIRED_XP.into(),
            next(current[0], &requirements.snaikka),
            next(current[1], &requirements.crayer),
            next(current[2], &requirements.cog),
            next(current[3], &requirements.hulk),
        ],
    );
}
