//! The church details page: what a "feeding the poor" donation to *this* town buys.
//!
//! Every figure comes from `p3_api::town::church`, which carries the reverse engineering;
//! this module is layout only.

use std::ffi::CStr;

use p3_api::{
    data::{class48::Class48Ptr, ddraw_set_constant_color, ddraw_set_text_mode, screen_rectangle::Rect, ui_render_text_at},
    game_world::GAME_WORLD_PTR,
    town::{
        beggars::{beggar_influx, beggar_influx_lifetime_days, beggar_target},
        church::{
            church_scale, donation_divisor, extension_cost, ChurchPtr, DECORATION_MAX,
            DECORATION_STEP_PER_SCALE, EXTENSION_MATERIALS, EXTENSION_STAGES, GATE_BEGGAR_INFLUX,
            GATE_GENEROUS, JEWELLERY_CAP_PER_SCALE, JEWELLERY_DECAY_PER_DAY, REPUTATION_PER_GOLD,
        },
        TOWN_FLAG_NO_BEGGAR_GROWTH,
    },
    ui::{font, rect_clipper_stuff, ui_church_window::UIChurchWindowPtr},
};

const BLACK: u32 = 0xFF00_0000;
const TEXT_MODE_LEFT: u32 = 1;
const TEXT_MODE_RIGHT: u32 = 2;

/// The page's first row, relative to the window's top edge, and the row pitch - matched to
/// the other details pages so the four look like one feature.
const FIRST_ROW_Y: i32 = 60;
const ROW_HEIGHT: i32 = 16;
/// A blank half-row between blocks, so the page reads as three groups rather than a list.
const GAP: i32 = ROW_HEIGHT / 2;

/// Text mode 2 draws right-aligned, so these are the RIGHT edges of each column. The label
/// column is left-aligned and names its left edge instead.
const LABEL_X: i32 = 40;
const VALUE_X: i32 = 250;
const NOTE_X: i32 = 270;

static HEADING: &CStr = c"Feeding the poor";
static CITIZENS: &CStr = c"Citizens";
static POOR_MOOD: &CStr = c"Poor satisfaction";
static GENEROUS: &CStr = c"\"Generous donation\"";
static INFLUX: &CStr = c"\"Beggars will come\"";
static PER_POINT: &CStr = c"1 reputation costs";
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

pub(crate) unsafe fn prepare_drawing_state() {
    let class48 = Class48Ptr::new();
    class48.set_ignore_below_gradient(0);
    class48.set_gradient_y(0);
}

pub(crate) unsafe fn invalidate(window: UIChurchWindowPtr) {
    let rect = Rect {
        left: window.get_x(),
        top: window.get_y(),
        right: window.get_x() + window.get_width(),
        bottom: window.get_y() + window.get_height(),
    };
    rect_clipper_stuff(&rect);
}

pub(crate) unsafe fn draw_page(window: UIChurchWindowPtr) {
    ddraw_set_constant_color(BLACK);
    ddraw_set_text_mode(TEXT_MODE_RIGHT);
    font::ddraw_set_font(font::get_normal_font());

    let x = window.get_x();
    let mut y = window.get_y() + FIRST_ROW_Y;
    let last_y = window.get_y() + window.get_height() - ROW_HEIGHT;

    draw_left(x + LABEL_X, y, HEADING.to_bytes());
    y += ROW_HEIGHT + GAP;

    let town_index = window.get_town_index();
    if !(0..0x100).contains(&town_index) {
        draw_left(x + LABEL_X, y, NO_TOWN.to_bytes());
        return;
    }
    let town = GAME_WORLD_PTR.get_town(town_index as u8);
    let citizens = town.get_citizens();
    let poor = town.get_poor_satisfaction();
    let divisor = donation_divisor(citizens, poor);

    // What the thresholds are made of, so the numbers below are not magic. Both drive the
    // divisor, and both are things the player can change.
    y = row(x, y, last_y, CITIZENS, &format!("{citizens}"), "");
    y = row(x, y, last_y, POOR_MOOD, &format!("{poor}"), "");
    y += GAP;

    // The two outcomes. The value is market value at THIS town's prices, which is the same
    // routine the donation dialog uses to price what you hand over.
    let beggars = town.get_beggars();
    let target = beggar_target(citizens, town.get_beggar_satisfaction());
    let blocked = town.get_flags() & TOWN_FLAG_NO_BEGGAR_GROWTH != 0;
    let influx = if blocked { 0 } else { beggar_influx(citizens, beggars, target) };

    y = row(
        x,
        y,
        last_y,
        GENEROUS,
        &format!("{} gold", GATE_GENEROUS * divisor),
        "reputation only",
    );
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
    y = row(
        x,
        y,
        last_y,
        INFLUX,
        &format!("{} gold", GATE_BEGGAR_INFLUX * divisor),
        &influx_note,
    );
    y += GAP;

    // Where the pool stands against its own equilibrium, so the influx figure above has
    // something to be read against.
    y = row(x, y, last_y, BEGGARS_NOW, &format!("{beggars}"), &format!("target {target}, donation does not raise it"));

    y += GAP;

    // The other two things the church takes money for. Both credit the same reputation as
    // feeding the poor; what differs is the cap, the decay and what the gold buys.
    if let Some(church) = ChurchPtr::of_town(town_index as u8) {
        let scale = church_scale();
        y = row(x, y, last_y, JEWELLERY, "", "");
        // The level saturates well below the cap, so both numbers are worth showing: past
        // the "full at" figure the gold still counts for reputation and nothing else.
        let step = DECORATION_STEP_PER_SCALE * scale;
        y = row(
            x,
            y,
            last_y,
            DECORATION,
            &format!("{} / {DECORATION_MAX}", church.decoration_level()),
            &format!("full at {} gold", (2 * DECORATION_MAX - 1) * step / 2),
        );
        y = row(
            x,
            y,
            last_y,
            COLLECTED,
            &format!("{} / {}", church.get_jewellery_money(), JEWELLERY_CAP_PER_SCALE * scale),
            &format!("decays {JEWELLERY_DECAY_PER_DAY}/day, gold past the cap is lost"),
        );
        y += GAP;

        let stage = church.get_extension_stage();
        y = row(x, y, last_y, EXTENSION, "", "");
        y = row(x, y, last_y, STAGE, &format!("{stage} / {EXTENSION_STAGES}"), "");
        match extension_cost(stage) {
            None => {
                y = row(x, y, last_y, FUNDING, "-", DONE.to_str().unwrap_or_default());
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
                y = row(x, y, last_y, FUNDING, &format!("{money} / {cost}"), &note);
                // Materials are drawn from the town's own stock, so a funded extension can
                // still stall - worth showing which ware is short.
                let held = church.get_extension_materials();
                let needed: Vec<String> = EXTENSION_MATERIALS
                    .iter()
                    .enumerate()
                    .map(|(i, (ware, per_stage))| {
                        format!("{ware:?} {}/{}", held[i], per_stage[stage as usize])
                    })
                    .collect();
                y = row(x, y, last_y, MATERIALS, "", &needed.join("  "));
            }
        }
        y += GAP;
    }

    // Reputation is linear and has no threshold, and all three donations credit it at the
    // same rate - so it belongs once, at the bottom.
    let gold_per_point = (1.0 / REPUTATION_PER_GOLD * church_scale() as f64).round() as i32;
    row(x, y, last_y, PER_POINT, &format!("{gold_per_point} gold"), "social, decays 1%/update");
}

/// One label/value/note row, or nothing once the window's bottom edge is reached.
unsafe fn row(x: i32, y: i32, last_y: i32, label: &CStr, value: &str, note: &str) -> i32 {
    if y > last_y {
        return y;
    }
    draw_left(x + LABEL_X, y, label.to_bytes());
    draw_text(x + VALUE_X, y, value.as_bytes());
    if !note.is_empty() {
        draw_left(x + NOTE_X, y, note.as_bytes());
    }
    y + ROW_HEIGHT
}

/// The game's text drawing takes a NUL-terminated string in its own codepage, so latin1
/// bytes go through unchanged.
unsafe fn draw_text(x: i32, y: i32, text: &[u8]) {
    let mut buffer = text.to_vec();
    buffer.push(0);
    ui_render_text_at(x, y, &buffer);
}

/// A left-aligned cell on a page that is otherwise right-aligned: switch, draw, switch
/// back, so the next right-aligned cell is not left-aligned by accident.
unsafe fn draw_left(x: i32, y: i32, text: &[u8]) {
    ddraw_set_text_mode(TEXT_MODE_LEFT);
    draw_text(x, y, text);
    ddraw_set_text_mode(TEXT_MODE_RIGHT);
}
