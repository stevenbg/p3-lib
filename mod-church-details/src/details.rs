//! The church details page: what the church's three actions in *this* town are worth.
//!
//! Every figure comes from `p3_api::town::church`, which carries the reverse engineering;
//! this module is layout only.

use std::ffi::CStr;

use p3_api::{
    game_world::GAME_WORLD_PTR,
    town::{
        beggars::{beggar_influx, beggar_influx_lifetime_days, beggar_target},
        church::{
            church_scale, donation_divisor, extension_cost, ChurchPtr, DECORATION_MAX, DECORATION_STEP_PER_SCALE, EXTENSION_MATERIALS,
            EXTENSION_STAGES, GATE_BEGGAR_INFLUX, GATE_GENEROUS, JEWELLERY_CAP_PER_SCALE, JEWELLERY_DECAY_PER_DAY, REPUTATION_PER_GOLD,
        },
        TOWN_FLAG_NO_BEGGAR_GROWTH,
    },
    ui::ui_church_window::UIChurchWindowPtr,
};
use p3_page::{Align, Cell, Column, Page, Symbol, Table};

/// The page's first row, relative to the window's top edge - matched to the other details
/// pages so the four look like one feature.
const FIRST_ROW_Y: i32 = 60;

/// Three columns: labels from the left, values ending at the value edge, notes from just past
/// it. The widths bound rich-text wrapping, which no single value reaches.
const LABEL: Column = Column::left(40, 200);
const VALUE: Column = Column::right(250, 120);
const NOTE: Column = Column::left(270, 230);

static HEADING: &CStr = c"Feeding the poor";
static REQUIRED: &CStr = c"Required value increases with the poor's satisfaction:";
static GENEROUS: &CStr = c"\"Generous donation\"";
static INFLUX: &CStr = c"\"Beggars will come\"";
static REPUTATION_SUM_A: &CStr = c"Your town reputation is a sum of local terms - buildings, tenants,";
static REPUTATION_SUM_B: &CStr = c"workers, outrigger, social, trading, spouse (hometown only) -";
static REPUTATION_SUM_C: &CStr = c"plus fleet size and wealth everywhere.";
static REPUTATION_DECAY: &CStr = c"Social and trading reputation decay 1%/day; the others don't.";
static BEGGARS_NOW: &CStr = c"Beggars";
static JEWELLERY: &CStr = c"Donations (jewellery)";
static DECORATION: &CStr = c"Decoration";
static COLLECTED: &CStr = c"Collected";
static EXTENSION: &CStr = c"Extension";
static STAGE: &CStr = c"Stage";
static FUNDING: &CStr = c"Funding";
static MATERIALS: &CStr = c"Materials";
static DONE: &CStr = c"fully extended";
static CLOSED: &CStr = c"not accepting donations";
static NO_TOWN: &CStr = c"no town";

fn page(window: &UIChurchWindowPtr) -> Page {
    Page::new(window, FIRST_ROW_Y)
}

pub(crate) unsafe fn invalidate(window: UIChurchWindowPtr) {
    page(&window).invalidate();
}

pub(crate) unsafe fn draw_page(window: UIChurchWindowPtr) {
    let page = page(&window);
    page.reset_state();
    let table = Table::new(&page, [LABEL, VALUE, NOTE]);
    let mut y = page.top;

    y = page.heading(LABEL.x, y, Align::Left, HEADING.to_bytes()) + page.gap();

    let town_index = window.get_town_index();
    if !(0..0x100).contains(&town_index) {
        page.line(LABEL.x, y, Align::Left, &NO_TOWN.into());
        return;
    }
    let town = GAME_WORLD_PTR.get_town(town_index as u8);
    let citizens = town.get_citizens();
    let poor = town.get_poor_satisfaction();
    let divisor = donation_divisor(citizens, poor);

    // Where the pool stands against its own equilibrium, first, so the influx figure below
    // has something to be read against.
    let beggars = town.get_beggars();
    let target = beggar_target(citizens, town.get_beggar_satisfaction());
    y = table.row(y, &[BEGGARS_NOW.into(), beggars.into(), format!("target {target}").into()]) + page.gap();

    // The two outcomes, under one heading. The value is market value at THIS town's prices,
    // which is the same routine the donation dialog uses to price what you hand over; the
    // divisor behind it is the town's size and its poor's satisfaction.
    y = page.line(LABEL.x, y, Align::Left, &REQUIRED.into());
    let blocked = town.get_flags() & TOWN_FLAG_NO_BEGGAR_GROWTH != 0;
    let influx = if blocked { 0 } else { beggar_influx(citizens, beggars, target) };

    y = table.row(y, &[GENEROUS.into(), Cell::amount(GATE_GENEROUS * divisor, Symbol::Coin), "reputation only".into()]);
    // Spell out what the influx is actually worth here rather than promising "beggars":
    // the jump is capped once the pool passes a quarter of the population, and a town
    // flagged 0x8 gets nothing at all.
    let influx_note = if blocked {
        "no beggars - town blocks growth".to_string()
    } else if influx == 0 {
        "no beggars - pool already full".to_string()
    } else {
        // The count alone reads as permanent, which it is not: an influx that overshoots
        // the target drains back. Say how long it lasts, so it reads as the window it is.
        match beggar_influx_lifetime_days(citizens, beggars, target, influx) {
            Some(days) => format!("+{influx}, {} hires, ~{days}d", influx / 4),
            None => format!("+{influx}, {} hires, stays", influx / 4),
        }
    };
    y = table.row(y, &[INFLUX.into(), Cell::amount(GATE_BEGGAR_INFLUX * divisor, Symbol::Coin), influx_note.into()]) + page.gap();

    // The other two things the church takes money for. Both credit the same reputation as
    // feeding the poor; what differs is the cap, the decay and what the gold buys.
    if let Some(church) = ChurchPtr::of_town(town_index as u8) {
        let scale = church_scale();
        y = page.heading(LABEL.x, y, Align::Left, JEWELLERY.to_bytes());
        // The level saturates well below the cap, so both numbers are worth showing: past
        // the "full at" figure the gold still counts for reputation and nothing else.
        let step = DECORATION_STEP_PER_SCALE * scale;
        let full_at = (2 * DECORATION_MAX - 1) * step / 2;
        y = table.row(
            y,
            &[
                DECORATION.into(),
                format!("{} / {DECORATION_MAX}", church.decoration_level()).into(),
                Cell::rich(format!("full at {full_at}{}", Symbol::Coin.escape())),
            ],
        );
        y = table.row(
            y,
            &[
                COLLECTED.into(),
                format!("{} / {}", church.get_jewellery_money(), JEWELLERY_CAP_PER_SCALE * scale).into(),
                format!("decays {JEWELLERY_DECAY_PER_DAY}/day").into(),
            ],
        ) + page.gap();

        let stage = church.get_extension_stage();
        y = page.heading(LABEL.x, y, Align::Left, EXTENSION.to_bytes());
        y = table.row(y, &[STAGE.into(), format!("{stage} / {EXTENSION_STAGES}").into(), Cell::Empty]);
        match extension_cost(stage) {
            None => {
                y = table.row(y, &[FUNDING.into(), "-".into(), DONE.into()]);
            }
            Some(cost) => {
                let money = church.get_extension_money();
                let note = if church.accepts_extension_money() {
                    format!("{} to go", cost - money)
                } else if money >= cost {
                    "funded, waiting on materials".to_string()
                } else {
                    CLOSED.to_str().unwrap_or_default().to_string()
                };
                y = table.row(y, &[FUNDING.into(), format!("{money} / {cost}").into(), note.into()]);
                // Materials are drawn from the town's own stock, so a funded extension can
                // still stall - worth showing which ware is short. One ware per line, under
                // the label.
                let held = church.get_extension_materials();
                for (i, (ware, per_stage)) in EXTENSION_MATERIALS.iter().enumerate() {
                    let label = if i == 0 { MATERIALS.into() } else { Cell::Empty };
                    let need = format!("{ware:?} {}/{}", held[i], per_stage[stage as usize]);
                    y = table.row(y, &[label, Cell::Empty, need.into()]);
                }
            }
        }
        y += page.gap();
    }

    // Reputation is linear and has no threshold, and all three donations credit it at the
    // same rate - so it is explained once, at the bottom, as prose.
    let gold_per_point = (1.0 / REPUTATION_PER_GOLD * church_scale() as f64).round() as i32;
    y = page.prose(
        LABEL.x,
        y,
        &Cell::rich(format!("All three actions grant +1 social reputation per {gold_per_point}{}.", Symbol::Coin.escape())),
    );
    for line in [REPUTATION_SUM_A, REPUTATION_SUM_B, REPUTATION_SUM_C, REPUTATION_DECAY] {
        y = page.prose(LABEL.x, y, &line.into());
    }
}
