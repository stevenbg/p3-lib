//! The trading-office window keys: F1 sets up the administrator's orders, CTRL/ALT+F1
//! provision and lock, DEL resets everything, CTRL/ALT+QWERTY reprice. Registration
//! lives in [crate::ffi] (the window's vtable open/close hooks); this is the actions.

use std::mem;

use log::{debug, info};
use num_traits::FromPrimitive;
use p3_api::{
    data::{enums::WareId, p3_ptr::P3Pointer},
    game_world::GAME_WORLD_PTR,
    operation::Operation,
    operations::{execute_operation, OPERATIONS_PTR},
    town::get_town_name,
    ui::ui_trading_office_window::UITradingOfficeWindowPtr,
};

use crate::prices::{buy_price, sell_price, PriceLevel};

/// What Alt+F1 provisions and locks: building materials (in-game units) covering any
/// building or ship.
const LOCKED_BUILDING_MATERIALS: [(WareId, i32); 6] = [
    (WareId::Cloth, 10),
    (WareId::Hemp, 8),
    (WareId::Pitch, 50),
    (WareId::Bricks, 80),
    (WareId::Timber, 50),
    (WareId::IronGoods, 50),
];

/// Amount set for buy orders whose current amount is 0, in the in-game display units the
/// office shows: loads for load wares, barrels for barrel wares. The two are the same
/// physical quantity - a load is ten barrels, so both scale to 40,000 raw - so a setup
/// asks for an equal amount of everything regardless of how it is measured.
const BUY_AMOUNT_LOADS: i32 = 20;
const BUY_AMOUNT_BARRELS: i32 = 200;
/// The administrator view of the trading office window ("Trading Office" side button, pages 0-6).
const ADMINISTRATOR_PAGE: i32 = 4;

/// Returns the administrator view's office and its index, or logs why not.
unsafe fn resolve_office() -> Option<(p3_api::data::office::OfficePtr, u16, String)> {
    let window = UITradingOfficeWindowPtr::new();
    if window.get_address() == 0 {
        crate::ffi::notify("Office keys: no trading office window open");
        return None;
    }
    let town_index = window.get_town_index();
    let town = get_town_name(town_index as u8).unwrap_or_else(|| "<unknown>".into());

    let page = window.get_selected_page();
    if page != ADMINISTRATOR_PAGE {
        // No popup: F1 doubles as mod-trading-office-details' "all towns" key, so
        // the office keys stay quiet on the other pages.
        debug!("office keys: {town} is on page {page}, not the Trading Office view");
        return None;
    }

    let merchant_index = OPERATIONS_PTR.get_player_merchant_index();
    let Some(office) = GAME_WORLD_PTR.get_office_in_of(town_index as _, merchant_index as _) else {
        crate::ffi::notify(&format!("Office keys: no player office found in {town}"));
        return None;
    };
    let office_index = (0..GAME_WORLD_PTR.get_offices_count()).find(|&i| GAME_WORLD_PTR.get_office(i).address == office.address)?;
    Some((office, office_index, town))
}

/// F1: for every ware whose current order is "do nothing", set BUY (at the Center buy
/// level, t1) if the town produces the ware, otherwise SELL (at the Center sell level,
/// t0 - the supply price). Wares that already have an order are left untouched; a buy's
/// amount is set to [BUY_AMOUNT_LOADS] / [BUY_AMOUNT_BARRELS] only if the current amount
/// is 0.
pub(crate) unsafe fn on_setup_hotkey() {
    let Some((office, office_index, town)) = resolve_office() else {
        return;
    };
    let town_index = UITradingOfficeWindowPtr::new().get_town_index() as u8;
    let production = GAME_WORLD_PTR.get_town(town_index).get_production_values();
    let current_prices = office.get_administrator_trade_prices();
    let stocks = office.get_administrator_trade_stock();

    let mut bought = Vec::new();
    let mut sold = 0;
    let mut untouched = 0;
    for ware_index in crate::ffi::TRADE_WARES {
        let ware_id = WareId::from_u16(ware_index).unwrap();
        let i = ware_index as usize;
        if current_prices[i] != 0 {
            untouched += 1;
            continue;
        }
        let (price, stock) = if production[i] > 0 {
            bought.push(format!("{ware_id:?}"));
            let stock = if stocks[i] == 0 {
                let amount = if ware_id.is_barrel_ware() { BUY_AMOUNT_BARRELS } else { BUY_AMOUNT_LOADS };
                amount.saturating_mul(ware_id.get_scaling())
            } else {
                stocks[i]
            };
            (-buy_price(ware_index, PriceLevel::Q), stock)
        } else {
            sold += 1;
            (sell_price(ware_index, PriceLevel::Q), stocks[i])
        };
        // Executed directly (not enqueued) so the view refresh below sees the new values.
        execute_operation(&Operation::OfficeAutotradeSettingChange {
            stock,
            price,
            office_index: office_index as _,
            ware_id,
        });
    }
    info!(
        "setup in {town}: buying produced wares [{}], selling {sold} others, {untouched} existing orders untouched",
        bought.join(", ")
    );
    crate::ffi::notify(&format!(
        "Office setup in {town}: {} buys, {sold} sells, {untouched} kept",
        bought.len()
    ));
    refresh_administrator_view();
}

/// Raise the administrator amounts of the given wares to the given raw amounts - never
/// lowering one - and lock their quantities (the "Lock min. store quantity for auto
/// trade ships" checkbox), so route ships cannot take the stock. Directions and prices
/// are untouched.
unsafe fn provision_and_lock(what: &str, targets: &[(WareId, i32)]) {
    let Some((office, office_index, town)) = resolve_office() else {
        return;
    };
    let town_index = UITradingOfficeWindowPtr::new().get_town_index() as u8;
    let prices = office.get_administrator_trade_prices();
    let stocks = office.get_administrator_trade_stock();
    let merchant_index = OPERATIONS_PTR.get_player_merchant_index();

    let mut raised = Vec::new();
    for &(ware_id, raw_amount) in targets {
        let i = ware_id as usize;
        if stocks[i] < raw_amount {
            execute_operation(&Operation::OfficeAutotradeSettingChange {
                stock: raw_amount,
                price: prices[i],
                office_index: office_index as _,
                ware_id,
            });
            raised.push(format!("{ware_id:?} {}", raw_amount / ware_id.get_scaling()));
        }
        execute_operation(&Operation::OfficeAutotradeLockChange {
            ware_id,
            town_index: town_index as u16,
            merchant_index: merchant_index as u16,
            lock: true,
        });
    }
    info!("{what} in {town}: locked {} wares, raised [{}]", targets.len(), raised.join(", "));
    crate::ffi::notify(&format!("Locked {what} in {town} ({} amounts raised)", raised.len()));
    refresh_administrator_view();
}

/// DEL in the office window: clear every auto-trade order back to nothing - no buy, no
/// sell, amount 0, and the "lock amount" checkbox unticked.
///
/// The inverse of [on_setup_hotkey], which is why it lives on the same window: setup only
/// ever fills in wares that have no order, so without a reset there is no way to undo it
/// short of twenty manual edits. A price of `0` is what the game itself means by "no
/// order" - [on_setup_hotkey] uses exactly that test to decide which wares it may touch.
///
/// This shadows the global route-clear DEL for as long as the window is open, the same way
/// the office F1 shadows the global route F1.
pub(crate) unsafe fn on_reset_office_hotkey() {
    let Some((office, office_index, town)) = resolve_office() else {
        return;
    };
    let town_index = UITradingOfficeWindowPtr::new().get_town_index() as u8;
    let merchant_index = OPERATIONS_PTR.get_player_merchant_index();
    let prices = office.get_administrator_trade_prices();
    let locks = office.get_administrator_trade_lock_bitmap();

    let mut cleared = 0;
    let mut unlocked = 0;
    for ware_index in crate::ffi::TRADE_WARES {
        let ware_id = WareId::from_u16(ware_index).unwrap();
        let i = ware_index as usize;
        if prices[i] != 0 {
            cleared += 1;
        }
        if locks & (1 << i) != 0 {
            unlocked += 1;
        }
        // Executed directly rather than enqueued, so the refresh below sees the new values.
        execute_operation(&Operation::OfficeAutotradeSettingChange {
            stock: 0,
            price: 0,
            office_index: office_index as _,
            ware_id,
        });
        execute_operation(&Operation::OfficeAutotradeLockChange {
            ware_id,
            town_index: town_index as u16,
            merchant_index: merchant_index as u16,
            lock: false,
        });
    }
    info!("reset in {town}: cleared {cleared} orders, unlocked {unlocked} amounts");
    crate::ffi::notify(&format!("Office reset in {town}: {cleared} orders cleared, {unlocked} unlocked"));
    refresh_administrator_view();
}

/// Ctrl+F1: provision and lock the celebration goods, at the stock a top-level
/// celebration requires - [p3_api::town::TownPtr::get_celebration_level_requirement],
/// rounded up to whole in-game units.
///
/// The requirement, not the doubled
/// [consumption](p3_api::town::TownPtr::get_celebration_consumption): the second half is
/// eaten without improving the celebration. Attendees are the whole citizen count on
/// purpose - the real figure is `citizens * ratio / 100` with the ratio capped at 99 - so
/// the amounts cover the best celebration the town could throw.
pub(crate) unsafe fn on_lock_staples_hotkey() {
    let town_index = UITradingOfficeWindowPtr::new().get_town_index() as u8;
    let town = GAME_WORLD_PTR.get_town(town_index);
    let attendees = town.get_citizens();
    let required = town.get_celebration_level_requirement(attendees);
    let targets: Vec<(WareId, i32)> = crate::ffi::TRADE_WARES
        .filter(|&ware_index| required[ware_index as usize] > 0)
        .map(|ware_index| {
            let ware_id = WareId::from_u16(ware_index).unwrap();
            let scaling = ware_id.get_scaling();
            (ware_id, (required[ware_index as usize] + scaling - 1) / scaling * scaling)
        })
        .collect();
    provision_and_lock(&format!("celebration goods for {attendees} guests"), &targets);
}

/// Alt+F1: provision and lock the [LOCKED_BUILDING_MATERIALS].
pub(crate) unsafe fn on_lock_building_materials_hotkey() {
    let targets = LOCKED_BUILDING_MATERIALS.map(|(ware_id, units)| (ware_id, units * ware_id.get_scaling()));
    provision_and_lock("building materials", &targets);
}

/// Sets the price of every ware that has an order, keeping its direction and amount:
/// sells get the sell level's price, buys the buy level's. A None level leaves that
/// direction untouched.
pub(crate) unsafe fn apply_prices(sell: Option<PriceLevel>, buy: Option<PriceLevel>) {
    let Some((office, office_index, town)) = resolve_office() else {
        return;
    };
    let current_prices = office.get_administrator_trade_prices();
    let stocks = office.get_administrator_trade_stock();

    let mut updated = 0;
    for ware_index in crate::ffi::TRADE_WARES {
        let i = ware_index as usize;
        let price = match current_prices[i] {
            0 => continue,
            p if p < 0 => match buy {
                Some(level) => -buy_price(ware_index, level),
                None => continue,
            },
            _ => match sell {
                Some(level) => sell_price(ware_index, level),
                None => continue,
            },
        };
        if price == current_prices[i] || price == 0 {
            continue;
        }
        execute_operation(&Operation::OfficeAutotradeSettingChange {
            stock: stocks[i],
            price,
            office_index: office_index as _,
            ware_id: WareId::from_u16(ware_index).unwrap(),
        });
        updated += 1;
    }
    let what = match (sell, buy) {
        (Some(level), _) => format!("Sell prices {level:?}"),
        (_, Some(level)) => format!("Buy prices {level:?}"),
        _ => "Prices".into(),
    };
    crate::ffi::notify(&format!("{what}: {updated} wares in {town}"));
    refresh_administrator_view();
}

/// The administrator view's amount rows: widget structs at window+0x9840 + row*0x190,
/// rows in the game's ware display order (the table at 0x698538, sorted at runtime by
/// localized name - the open method's populate loop at 0x5d8f40 reads it the same way).
/// The number widget setter 0x45c930 clamps to [row+0x180]..[row+0x184], stores the
/// value at row+0x188 and refreshes the label text.
const OFFICE_ROW_BASE: u32 = 0x9840;
const OFFICE_ROW_STRIDE: u32 = 0x190;
const WARE_DISPLAY_ORDER: *const u8 = 0x00698538 as _;
const WIDGET_SET_VALUE: u32 = 0x0045c930;

/// Re-selects the administrator page (rebuilds the direction arrows and prices) and
/// pushes the office's amounts into the row widgets the way the window's own open
/// method does - the amounts are populated only there, which is why they never
/// refreshed before. (Cycling the window's close+open repopulates too, but detaches
/// the side menu: the game's real open path goes through a view controller.)
unsafe fn refresh_administrator_view() {
    let window = UITradingOfficeWindowPtr::new();
    if window.get_address() == 0 {
        return;
    }
    window.select_new_page(ADMINISTRATOR_PAGE);

    let merchant_index = OPERATIONS_PTR.get_player_merchant_index();
    let Some(office) = GAME_WORLD_PTR.get_office_in_of(window.get_town_index() as _, merchant_index as _) else {
        return;
    };
    let stocks = office.get_administrator_trade_stock();
    let set_value: extern "thiscall" fn(u32, i32) = mem::transmute(WIDGET_SET_VALUE);
    for row in 0..crate::ffi::TRADE_WARES.end as u32 {
        let ware = *WARE_DISPLAY_ORDER.add(row as usize) as usize;
        if ware >= crate::ffi::TRADE_WARES.end as usize {
            continue;
        }
        let scaling = WareId::from_usize(ware).unwrap().get_scaling();
        let value = (stocks[ware] / scaling).clamp(0, 9999);
        set_value(window.get_address() + OFFICE_ROW_BASE + row * OFFICE_ROW_STRIDE, value);
    }
}
