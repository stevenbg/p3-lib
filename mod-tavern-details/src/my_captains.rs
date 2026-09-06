//! Key 3: the player's own captains, one row per ship that has one - the ship's name and
//! the captain's three skills under the same icons the crew table uses, and his wage. A ship
//! without a captain is left out; so is a captain-less fleet, which simply draws the heading
//! and nothing under it. With alt, each skill shows its ceiling too, `level/cap`: the caps
//! belong to the record's slot (`p3_api::auto_trader::skill_caps`), so two captains with the
//! same level can have different room to grow.
//!
//! The fleet is the merchant's own ship chain (`merchant+0xE` head, `ship+0x4` next), so
//! convoy members and ships at sea are included - a captain trains wherever his ship is.
//! The walk is bounded by the ship count, since a corrupt link would otherwise spin. The
//! chain links the newest ship first; the table is sorted by the header a click selected.
//!
//! A fleet with more captains than the page has rows gets the game's own scrollbar
//! (`p3_api::ui::scroll_list`): it is registered in the scene's root container after the
//! tavern window while this view is on screen, so the game draws it above the page, drags
//! its thumb and feeds it the mouse wheel over the table; the table starts at the row the
//! bar points to. The page repaints every frame anyway (see `details::invalidate`), so no
//! extra invalidation is needed when the bar moves.
//!
//! Layout: the table hugs the window's right edge - the bar against the frame, the columns
//! to its left - and stops a row short of the hint line so the bar clears the window's
//! close button in the bottom-right corner.
use std::{
    cmp::Ordering as Order,
    ffi::CStr,
    sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering},
};

use log::info;
use p3_api::{
    auto_trader::{skill_caps, AutoTraderPtr, SKILL_PER_LEVEL},
    data::screen_rectangle::Rect,
    game_world::GAME_WORLD_PTR,
    operations::OPERATIONS_PTR,
    ships::ShipsPtr,
    ui::{
        graphics::{GRAPHIC_ID_BONUS, GRAPHIC_ID_MONEY},
        scroll_list::ScrollList,
        ui_tavern_window::UITavernWindowPtr,
    },
};
use p3_ui::{Cell, Column, Page, Table};

use crate::details::page;

/// The bar's right edge from the window's right edge: inside the frame.
const RIGHT_MARGIN: i32 = 12;
/// The bar's width (its buttons are 27 px wide, `set_rect` places it by them).
const BAR_WIDTH: i32 = 27;
/// Between the pay column's digits and the bar.
const BAR_GAP: i32 = 8;
/// Between the right-aligned value columns, which is also how wide each is.
const COLUMN_GAP: i32 = 36;
/// How far left of the name column its cell (and the wheel area) reaches.
const NAME_CELL_WIDTH: i32 = 120;

/// The table's columns, laid out from the window's right edge inwards so the bar sits
/// against the frame; also the bar's window-relative right edge.
fn columns(page: &Page) -> ([Column; 5], i32) {
    let bar_right = page.width - RIGHT_MARGIN;
    let pay = bar_right - BAR_WIDTH - BAR_GAP;
    let columns = [
        Column::right(pay - 4 * COLUMN_GAP, NAME_CELL_WIDTH),
        Column::right(pay - 3 * COLUMN_GAP, COLUMN_GAP),
        Column::right(pay - 2 * COLUMN_GAP, COLUMN_GAP),
        Column::right(pay - COLUMN_GAP, COLUMN_GAP),
        Column::right(pay, COLUMN_GAP),
    ];
    (columns, bar_right)
}

/// The table ends one row above the page's last table row, so the bar (which runs the
/// table's full height at the right edge) clears the close button.
fn table(page: &Page) -> (Table<'_>, i32) {
    let (columns, bar_right) = columns(page);
    (Table::new(page, columns).ending_at(page.last_y - page.row_height), bar_right)
}

/// The scrollbar's object, built on first use; 0 before that.
static LIST: AtomicU32 = AtomicU32::new(0);

/// The sort, kept across openings like the view: a click on a column header selects it
/// (name ascending, numbers descending), a second click reverses it. The column numbers are
/// the table's column indices.
static SORT_COLUMN: AtomicU8 = AtomicU8::new(COLUMN_NAME);
static SORT_REVERSED: AtomicBool = AtomicBool::new(false);
const COLUMN_NAME: u8 = 0;
const COLUMN_TRADE: u8 = 1;
const COLUMN_NAVIGATION: u8 = 2;
const COLUMN_COMBAT: u8 = 3;
const COLUMN_WAGE: u8 = 4;

/// Per frame from the tavern window's update, on every page: keep the bar attached exactly
/// while this view is on screen, keep its row count current, and read its position.
pub(crate) unsafe fn update(window: UITavernWindowPtr, active: bool) {
    if !active {
        detach();
        return;
    }
    let list = match LIST.load(Ordering::Relaxed) {
        0 => {
            let list = ScrollList::new();
            LIST.store(list.address, Ordering::Relaxed);
            info!("my captains: scrollbar built at {:#x}", list.address);
            list
        }
        address => ScrollList { address },
    };
    if !list.is_attached() {
        let page = page(&window);
        let area = table_area(&page);
        list.set_area(area, page.row_height);
        list.attach_to_root();
        list.set_count(own_captains().len() as u32);
        info!(
            "my captains: bar attached - area {},{}..{},{}, {} captains, {} rows fit",
            area.left,
            area.top,
            area.right,
            area.bottom,
            own_captains().len(),
            list.visible_rows()
        );
    }
    list.set_count(own_captains().len() as u32);
    list.poll();
}

/// Take the bar off the screen: on leaving the view or the page, and when the window
/// closes (which stops the update calls that would otherwise notice).
pub(crate) unsafe fn detach() {
    let address = LIST.load(Ordering::Relaxed);
    if address != 0 {
        let list = ScrollList { address };
        if list.is_attached() {
            list.detach();
        }
    }
}

/// The rows under the heading, down to the table's last row; the bar sits against its
/// right edge and the wheel works anywhere inside it.
fn table_area(page: &Page) -> Rect {
    let (table, bar_right) = table(page);
    let (left, _) = table.extent();
    Rect {
        left: page.abs_x(left),
        top: page.abs_y(page.top + page.row_height),
        right: page.abs_x(bar_right),
        bottom: page.abs_y(table.last_y + page.row_height),
    }
}

/// A left click at screen `x`,`y`: a header cell selects or reverses the sort.
pub(crate) unsafe fn on_click(window: UITavernWindowPtr, x: i32, y: i32) {
    let page = page(&window);
    let (table, _) = table(&page);
    let Some(column) = table.hit_test(x, y, page.top) else {
        return;
    };
    let column = column as u8;
    if SORT_COLUMN.load(Ordering::Relaxed) == column {
        SORT_REVERSED.fetch_xor(true, Ordering::Relaxed);
    } else {
        SORT_COLUMN.store(column, Ordering::Relaxed);
        SORT_REVERSED.store(false, Ordering::Relaxed);
    }
    info!("my captains: sort by column {column}, reversed {}", SORT_REVERSED.load(Ordering::Relaxed));
}

/// The sort order's natural direction: names ascending, numbers descending; a repeat click
/// reverses it.
fn compare(column: u8, a: &(Vec<u8>, Captain), b: &(Vec<u8>, Captain)) -> Order {
    let order = match column {
        COLUMN_NAME => a.0.to_ascii_lowercase().cmp(&b.0.to_ascii_lowercase()),
        COLUMN_TRADE => b.1.ptr.get_trade_skill().cmp(&a.1.ptr.get_trade_skill()),
        COLUMN_NAVIGATION => b.1.ptr.get_navigation_skill().cmp(&a.1.ptr.get_navigation_skill()),
        COLUMN_COMBAT => b.1.ptr.get_combat_skill().cmp(&a.1.ptr.get_combat_skill()),
        COLUMN_WAGE => b.1.ptr.get_daily_wage().cmp(&a.1.ptr.get_daily_wage()),
        _ => Order::Equal,
    };
    if SORT_REVERSED.load(Ordering::Relaxed) {
        order.reverse()
    } else {
        order
    }
}

/// The heading and the rows from the bar's first visible row on; `show_caps` (alt) writes
/// every skill as `level/cap`. Returns the y below the last row drawn, like the other tables.
pub(crate) unsafe fn draw(page: &Page, heading: &CStr, show_caps: bool) -> i32 {
    let (table, _) = table(page);
    let sort = (SORT_COLUMN.load(Ordering::Relaxed) as usize, SORT_REVERSED.load(Ordering::Relaxed));
    let mut y = table.header(
        page.top,
        &[
            heading.into(),
            Cell::graphic_frame(GRAPHIC_ID_BONUS, 0),
            Cell::graphic_frame(GRAPHIC_ID_BONUS, 1),
            Cell::graphic_frame(GRAPHIC_ID_BONUS, 2),
            Cell::graphic(GRAPHIC_ID_MONEY),
        ],
        Some(sort),
    );

    let first_row = match LIST.load(Ordering::Relaxed) {
        0 => 0,
        address => ScrollList { address }.first_row() as usize,
    };
    for (name, trader) in own_captains().into_iter().skip(first_row) {
        if !table.fits(y) {
            break;
        }
        // Same order as the game's captain panel: trade, navigation, combat. The caps come
        // back as (navigation, trade, combat).
        let (navigation_cap, trade_cap, combat_cap) = skill_caps(trader.index);
        let [trade, navigation, combat] = [
            (trader.ptr.get_trade_skill(), trade_cap),
            (trader.ptr.get_navigation_skill(), navigation_cap),
            (trader.ptr.get_combat_skill(), combat_cap),
        ]
        .map(|(skill, cap)| {
            let level = AutoTraderPtr::skill_level(skill);
            if show_caps {
                Cell::from(format!("{level}/{}", cap / SKILL_PER_LEVEL))
            } else {
                Cell::number(level as i32)
            }
        });
        y = table.row(y, &[name.into(), trade, navigation, combat, Cell::number(trader.ptr.get_daily_wage() as i32)]);
    }
    y
}

/// A captain record with the array index its caps hang off.
#[derive(Clone, Copy)]
struct Captain {
    index: u16,
    ptr: AutoTraderPtr,
}

/// The player's captained ships in the selected sort order (stable, so ties keep the
/// chain order): the ship's name (its fixed 32-byte buffer stripped of padding) and the
/// captain record.
unsafe fn own_captains() -> Vec<(Vec<u8>, Captain)> {
    let mut captains = captains_in_chain_order();
    let column = SORT_COLUMN.load(Ordering::Relaxed);
    captains.sort_by(|a, b| compare(column, a, b));
    captains
}

/// The fleet as the merchant's ship chain links it: the newest ship first.
unsafe fn captains_in_chain_order() -> Vec<(Vec<u8>, Captain)> {
    let ships = ShipsPtr::new();
    let merchant = GAME_WORLD_PTR.get_merchant(OPERATIONS_PTR.get_player_merchant_index() as u16);
    let mut captains = Vec::new();
    let mut ship_index = merchant.get_first_ship_index();
    for _ in 0..ships.get_ships_size() {
        let Some(ship) = ships.get_ship(ship_index) else {
            break;
        };
        let index = ship.get_captain_index();
        if let Some(ptr) = ships.get_auto_trader(index) {
            captains.push((ship.get_name().trim_end_matches('\0').as_bytes().to_vec(), Captain { index, ptr }));
        }
        ship_index = ship.get_next_ship_index_of_merchant();
    }
    captains
}
