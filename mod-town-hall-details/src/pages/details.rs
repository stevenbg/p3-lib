use num_traits::cast::FromPrimitive;
use p3_api::{
    data::{ddraw_set_constant_color, ddraw_set_text_mode, enums::WareId, fill_p3_string, render_window_title, ui_render_text_at},
    game_world::GAME_WORLD_PTR,
    ui::{
        font,
        rich_text::{draw_rich_text, TOWN_HALL_WINDOW_LAYOUT_OFFSET},
        ui_town_hall_window::UITownHallWindowPtr,
    },
};
use std::ffi::{CStr, CString};

const COL_OFFSETS: &[i32; 4] = &[90, 180, 270, 360];
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

    fn unit_symbol(&self) -> &'static str {
        if self.ware_id().is_barrel_ware() {
            "B"
        } else {
            "L"
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

    ddraw_set_constant_color(0xff000000);
    ddraw_set_text_mode(2);
    let x = window.get_x();
    let mut y = window.get_y() + 60;

    font::ddraw_set_font(font::get_header_font());
    ui_render_text_at(x + COL_OFFSETS[0], y, GOODS.to_bytes());
    ui_render_text_at(x + COL_OFFSETS[1], y, STOCK.to_bytes());
    ui_render_text_at(x + COL_OFFSETS[2], y, CONSUMPTION.to_bytes());
    ui_render_text_at(x + COL_OFFSETS[3], y, DAYS.to_bytes());
    y += 20;

    let mut effective_prods_count = 0;
    font::ddraw_set_font(font::get_normal_font());
    let mut has_meat_or_leather = false;
    for data in &hanse_data {
        let ware = data.ware_id();
        let days = data.get_days();
        let mut color = if !data.has_consumption() {
            0xFFD3D3D3
        } else if effective_prods_count < 3 {
            if ware == WareId::Meat || ware == WareId::Leather {
                if !has_meat_or_leather {
                    effective_prods_count += 1;
                    has_meat_or_leather = true;
                }
            } else {
                effective_prods_count += 1;
            }
            0xFF7CFC00
        } else {
            0xFF000000
        };
        if ware == WareId::Spices {
            color = 0xFF660000;
        }
        ddraw_set_constant_color(color);
        ui_render_text_at(x + COL_OFFSETS[0], y, CString::new(format!("{ware:?}")).unwrap().to_bytes());
        ui_render_text_at(x + COL_OFFSETS[3], y, CString::new(format!("{days}")).unwrap().to_bytes());

        // Stock and consumption carry the game's own load/barrel symbol, which only the
        // rich-text pass can splice in. `\r` right-aligns with x as the cell's right
        // edge, matching the plain cells above; the pass sets its own font and colour,
        // so both are restored for the next row.
        let scaling = data.scaling();
        let symbol = data.unit_symbol();
        let stock_cell = CString::new(format!("\\r{}\\{symbol}", data.total_wares / scaling)).unwrap();
        let consumption_cell = CString::new(format!("\\r{}\\{symbol}", data.total_consumption / scaling)).unwrap();
        let layout = window.address + TOWN_HALL_WINDOW_LAYOUT_OFFSET;
        draw_rich_text(layout, stock_cell.as_bytes_with_nul(), x + COL_OFFSETS[1], y, 150, 20, color);
        draw_rich_text(layout, consumption_cell.as_bytes_with_nul(), x + COL_OFFSETS[2], y, 150, 20, color);
        ddraw_set_text_mode(2);
        font::ddraw_set_font(font::get_normal_font());

        y += 20;
    }
}
