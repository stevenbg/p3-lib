//! The church details page: what a "feeding the poor" donation to *this* town buys.
//!
//! Every figure comes from `p3_api::town::church`, which carries the reverse engineering;
//! this module is layout only.

use std::ffi::CStr;

use p3_api::ui::rich_text::{draw_rich_text, font_escape, CHURCH_WINDOW_LAYOUT_OFFSET};

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
    let layout = window.address + CHURCH_WINDOW_LAYOUT_OFFSET;
    let mut y = window.get_y() + FIRST_ROW_Y;
    let last_y = window.get_y() + window.get_height() - ROW_HEIGHT;

    heading(x, y, HEADING);
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

    // Where the pool stands against its own equilibrium, first, so the influx figure below
    // has something to be read against.
    let beggars = town.get_beggars();
    let target = beggar_target(citizens, town.get_beggar_satisfaction());
    y = row(x, y, last_y, BEGGARS_NOW, &format!("{beggars}"), &format!("target {target}"));
    y += GAP;

    // The two outcomes, under one heading. The value is market value at THIS town's prices,
    // which is the same routine the donation dialog uses to price what you hand over; the
    // divisor behind it is the town's size and its poor's satisfaction.
    draw_left(x + LABEL_X, y, REQUIRED.to_bytes());
    y += ROW_HEIGHT;
    let blocked = town.get_flags() & TOWN_FLAG_NO_BEGGAR_GROWTH != 0;
    let influx = if blocked { 0 } else { beggar_influx(citizens, beggars, target) };

    y = row_gold(layout, x, y, last_y, GENEROUS, GATE_GENEROUS * divisor, "reputation only");
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
    y = row_gold(layout, x, y, last_y, INFLUX, GATE_BEGGAR_INFLUX * divisor, &influx_note);
    y += GAP;

    // The other two things the church takes money for. Both credit the same reputation as
    // feeding the poor; what differs is the cap, the decay and what the gold buys.
    if let Some(church) = ChurchPtr::of_town(town_index as u8) {
        let scale = church_scale();
        y = heading_row(x, y, last_y, JEWELLERY);
        // The level saturates well below the cap, so both numbers are worth showing: past
        // the "full at" figure the gold still counts for reputation and nothing else.
        let step = DECORATION_STEP_PER_SCALE * scale;
        let row_y = y;
        y = row(x, y, last_y, DECORATION, &format!("{} / {DECORATION_MAX}", church.decoration_level()), "");
        if row_y <= last_y {
            draw_rich_left(layout, x + NOTE_X, row_y, &format!("full at {}\\C", (2 * DECORATION_MAX - 1) * step / 2));
        }
        y = row(
            x,
            y,
            last_y,
            COLLECTED,
            &format!("{} / {}", church.get_jewellery_money(), JEWELLERY_CAP_PER_SCALE * scale),
            &format!("decays {JEWELLERY_DECAY_PER_DAY}/day"),
        );
        y += GAP;

        let stage = church.get_extension_stage();
        y = heading_row(x, y, last_y, EXTENSION);
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
                // One ware per line, under the label.
                for (i, need) in needed.iter().enumerate() {
                    let label = if i == 0 { MATERIALS } else { c"" };
                    y = row(x, y, last_y, label, "", need);
                }
            }
        }
        y += GAP;
    }

    // Reputation is linear and has no threshold, and all three donations credit it at the
    // same rate - so it is explained once, at the bottom, as prose.
    let gold_per_point = (1.0 / REPUTATION_PER_GOLD * church_scale() as f64).round() as i32;
    if y <= last_y {
        draw_rich_left(
            layout,
            x + LABEL_X,
            y,
            &format!("All three actions grant +1 social reputation per {gold_per_point}\\C."),
        );
    }
    y += ROW_HEIGHT;
    for line in [REPUTATION_SUM_A, REPUTATION_SUM_B, REPUTATION_SUM_C, REPUTATION_DECAY] {
        y = text_row(x, y, last_y, line);
    }
}

/// A line of prose starting at the label column; advances like [row].
unsafe fn text_row(x: i32, y: i32, last_y: i32, text: &CStr) -> i32 {
    if y > last_y {
        return y;
    }
    draw_left(x + LABEL_X, y, text.to_bytes());
    y + ROW_HEIGHT
}

/// [row] with a gold value drawn as `<n>` plus the game's coin symbol.
unsafe fn row_gold(layout: u32, x: i32, y: i32, last_y: i32, label: &CStr, gold: i32, note: &str) -> i32 {
    if y > last_y {
        return y;
    }
    draw_left(x + LABEL_X, y, label.to_bytes());
    draw_rich_right(layout, x + VALUE_X, y, &format!("{gold}\\C"));
    if !note.is_empty() {
        draw_left(x + NOTE_X, y, note.as_bytes());
    }
    y + ROW_HEIGHT
}

/// Text with markup - the coin symbol is the escape `\C`, not a glyph - through the
/// framework's rich-text pass, right-aligned: `\r` anchors the line's right edge at `x`,
/// like the page's plain values. That pass ignores the ddraw font and would default to the
/// heading face, so the line selects the body font first; it also sets its own colour and
/// text mode, so the page's state is restored after it.
unsafe fn draw_rich_right(layout: u32, x: i32, y: i32, text: &str) {
    draw_rich(layout, x, y, &format!("{}\\r{text}", font_escape(font::NORMAL_FONT_INDEX)));
}

/// The same, left-aligned from `x`.
unsafe fn draw_rich_left(layout: u32, x: i32, y: i32, text: &str) {
    draw_rich(layout, x, y, &format!("{}\\l{text}", font_escape(font::NORMAL_FONT_INDEX)));
}

/// The rich-text pass wraps at `width`; the page's content spans the window from the label
/// column to the right margin, and no line on it is wider than this.
const RICH_WRAP_WIDTH: i32 = 460;

unsafe fn draw_rich(layout: u32, x: i32, y: i32, text: &str) {
    let mut bytes = text.as_bytes().to_vec();
    bytes.push(0);
    draw_rich_text(layout, &bytes, x, y, RICH_WRAP_WIDTH, ROW_HEIGHT, BLACK);
    ddraw_set_constant_color(BLACK);
    ddraw_set_text_mode(TEXT_MODE_RIGHT);
    font::ddraw_set_font(font::get_normal_font());
}

/// A section heading in the game's heading face (the Black weight of the body font), then
/// back to the body font for the rows under it.
unsafe fn heading(x: i32, y: i32, label: &CStr) {
    font::ddraw_set_font(font::get_header_font());
    draw_left(x + LABEL_X, y, label.to_bytes());
    font::ddraw_set_font(font::get_normal_font());
}

/// [heading] as a row: advances like [row], or nothing past the window's bottom edge.
unsafe fn heading_row(x: i32, y: i32, last_y: i32, label: &CStr) -> i32 {
    if y > last_y {
        return y;
    }
    heading(x, y, label);
    y + ROW_HEIGHT
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
