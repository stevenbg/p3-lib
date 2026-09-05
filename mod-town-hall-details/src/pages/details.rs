use num_traits::cast::FromPrimitive;
use p3_api::{
    data::{enums::WareId, fill_p3_string, render_window_title},
    game_world::GAME_WORLD_PTR,
    ui::ui_town_hall_window::UITownHallWindowPtr,
};
use p3_page::{Cell, Column, Page, Symbol, Table};
use std::ffi::CStr;

/// Under the window's title banner.
const FIRST_ROW_Y: i32 = 60;
/// Wider than the other details pages, matching the game's own tables on this window.
const ROW_HEIGHT: i32 = 20;
/// Four right-aligned columns; the width bounds the rich-text cells' wrapping.
const COLUMNS: [Column; 4] = [Column::right(90, 90), Column::right(180, 90), Column::right(270, 90), Column::right(360, 90)];

/// Nobody consumes the ware.
const GREY: u32 = 0xFFD3_D3D3;
/// One of the three wares the Hanse runs out of first.
const GREEN: u32 = 0xFF7C_FC00;
/// Spices are imported, never produced.
const DARK_RED: u32 = 0xFF66_0000;

pub static TITLE: &CStr = c"Details";
pub static GOODS: &CStr = c"Goods";
pub static STOCK: &CStr = c"Stock";
pub static CONSUMPTION: &CStr = c"Consumption";
pub static DAYS: &CStr = c"Days";

#[derive(Debug, Default)]
struct HanseaticWareData {
    total_wares: i32,
    total_consumption: i32,
    ware: usize,
}

impl HanseaticWareData {
    fn ware_id(&self) -> WareId {
        WareId::from_usize(self.ware).unwrap()
    }

    /// Raw units per load (2000) or per barrel (200), see WareId::get_scaling.
    fn scaling(&self) -> i32 {
        self.ware_id().get_scaling()
    }

    fn symbol(&self) -> Symbol {
        if self.ware_id().is_barrel_ware() {
            Symbol::Barrel
        } else {
            Symbol::Load
        }
    }

    /// Consumption below one load/barrel a day is treated as nobody consuming it.
    fn has_consumption(&self) -> bool {
        self.total_consumption >= self.scaling()
    }

    /// Both stock and consumption are raw units (per day), so this is
    /// how many days the Hanse-wide stock lasts at the current consumption.
    fn get_days(&self) -> i32 {
        if self.has_consumption() {
            self.total_wares / self.total_consumption
        } else {
            0
        }
    }
}

pub(crate) unsafe fn draw_page(window: UITownHallWindowPtr) {
    let mut hanse_data: [HanseaticWareData; 20] = Default::default();
    let towns_count = GAME_WORLD_PTR.get_towns_count();

    for town_index in 0..towns_count {
        let town = GAME_WORLD_PTR.get_town(town_index as _);
        let stock = town.get_storage().get_wares();
        let consumption_citizens = town.get_daily_consumptions_citizens();
        let consumption_businesses = town.get_storage().get_daily_consumptions_businesses();
        let unknown_stock = town.get_unknown_stock();
        for ware in 0..20 {
            hanse_data[ware].total_wares += stock[ware] + unknown_stock[ware];
            hanse_data[ware].total_consumption += consumption_citizens[ware];
            hanse_data[ware].total_consumption += consumption_businesses[ware];
            hanse_data[ware].ware = ware;
        }

        let mut office_index = town.get_first_office_index();
        while office_index < GAME_WORLD_PTR.get_offices_count() {
            let office = GAME_WORLD_PTR.get_office(office_index);
            let office_stock = office.get_storage().get_wares();
            let office_consumption = office.get_storage().get_daily_consumptions_businesses();
            for ware in 0..20 {
                hanse_data[ware].total_wares += office_stock[ware];
                hanse_data[ware].total_consumption += office_consumption[ware];
            }
            office_index = office.get_next_office_in_town_index();
        }
    }
    hanse_data.sort_by_key(|a| a.get_days());

    let mut title_p3_string: u32 = 0;
    fill_p3_string((&mut title_p3_string) as *mut _ as _, TITLE.to_bytes());
    render_window_title(title_p3_string as _, window.address as _);

    let mut page = Page::new(&window, FIRST_ROW_Y);
    page.row_height = ROW_HEIGHT;
    page.reset_state();
    let table = Table::new(&page, COLUMNS);
    let mut y = table.header(page.top, &[GOODS.into(), STOCK.into(), CONSUMPTION.into(), DAYS.into()], None);

    // The three wares that run out first are the effective productions to build; meat and
    // leather come from one building, so together they count once.
    let mut effective_prods_count = 0;
    let mut has_meat_or_leather = false;
    for data in &hanse_data {
        let ware = data.ware_id();
        let mut color = if !data.has_consumption() {
            GREY
        } else if effective_prods_count < 3 {
            if ware == WareId::Meat || ware == WareId::Leather {
                if !has_meat_or_leather {
                    effective_prods_count += 1;
                    has_meat_or_leather = true;
                }
            } else {
                effective_prods_count += 1;
            }
            GREEN
        } else {
            p3_page::BLACK
        };
        if ware == WareId::Spices {
            color = DARK_RED;
        }
        let scaling = data.scaling();
        y = table.row_colored(
            y,
            &[
                format!("{ware:?}").into(),
                Cell::amount(data.total_wares / scaling, data.symbol()),
                Cell::amount(data.total_consumption / scaling, data.symbol()),
                data.get_days().into(),
            ],
            color,
        );
    }
}
