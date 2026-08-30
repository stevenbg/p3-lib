//! The goods dialog ("Automatic maritime trading"): F1 fills the displayed stop, the
//! CTRL/ALT+QWERTY keys reprice it in place. Key registration and the dialog's
//! open/close hooks live in [crate::ffi]; this is the dialog knowledge and the actions.

use std::mem;

use log::info;
use num_traits::FromPrimitive;
use p3_api::{data::enums::WareId, game_world::GAME_WORLD_PTR, town::get_town_name};
use p3_rou::builder;

use crate::prices::{buy_price, sell_price, PriceLevel};

/// The "Automatic maritime trading" (goods) dialog object, constructed at startup
/// (ctor 0x403020, vtable 0x66a7f0, allocated by the mass-constructor at 0x424eb0).
/// Its +0xa4 holds the POOL INDEX of the displayed stop - its arrows walk the pool
/// chain through it - and the close method (vtable+0x118 = 0x4066f0) sets it to -1,
/// so one field carries both "is open" and "which stop".
pub(crate) const DIALOG_PTR: *const u32 = 0x006cba74 as _;
const DIALOG_STOP_INDEX_OFFSET: u32 = 0xa4;
const DIALOG_SHIP_INDEX_OFFSET: u32 = 0xa8;
const DIALOG_FLAG_OFFSET: u32 = 0x547c;
/// thiscall(this, stop_pool_index, ship_index, flag): populate the dialog's widgets
/// (direction arrows, amount and price texts) from the stop record. Called by the
/// dialog's own arrows and by the panel's Goods button (0x48c432).
pub(crate) const DIALOG_POPULATE: u32 = 0x00405a20;
/// Populate's three call sites (module-relative, as hook_call_rel32 takes them): the
/// dialog's own arrows (0x004075E3, 0x0040763A) and the panel's Goods button
/// (0x0048C432). Wrapping them is the dialog's "open" event.
pub(crate) const DIALOG_POPULATE_CALL_OFFSETS: [u32; 3] = [0x75E3, 0x763A, 0x8C432];
/// The dialog's close, vtable slot +0x118 of 0x0066A7F0 (module-relative pointer
/// location, as hook_function_pointer takes it).
pub(crate) const DIALOG_CLOSE_POINTER_OFFSET: u32 = 0x26A7F0 + 0x118;

/// The goods dialog and the pool index of the stop it displays, if it is open.
pub(crate) unsafe fn goods_dialog_stop() -> Option<(u32, u32)> {
    let dialog = *DIALOG_PTR;
    if dialog == 0 {
        return None;
    }
    let stop_index = *((dialog + DIALOG_STOP_INDEX_OFFSET) as *const i32);
    if stop_index < 0 || stop_index as u16 >= *crate::routes::ROUTE_STOP_POOL_COUNT {
        return None;
    }
    Some((dialog, stop_index as u32))
}

/// Repopulate the goods dialog from its stop record, the way its own arrows do. The
/// displayed texts are sprintf-cached in the dialog object, so in-place pool writes
/// stay invisible without this.
unsafe fn refresh_goods_dialog(dialog: u32, stop_index: u32) {
    let populate: extern "thiscall" fn(u32, u32, u32, u32) = mem::transmute(DIALOG_POPULATE);
    let ship_index = *((dialog + DIALOG_SHIP_INDEX_OFFSET) as *const u32);
    let flag = *((dialog + DIALOG_FLAG_OFFSET) as *const u8) as u32;
    populate(dialog, stop_index, ship_index, flag);
    // This direct call bypasses the three hooked call sites, but populate still
    // closes the open dialog internally - which unregisters the dialog keys through
    // the close hook. Re-register, exactly like the hooked sites do post-call;
    // without this, the first dialog hotkey (whose handler refreshes the dialog)
    // silently disarms all the others.
    crate::ffi::reregister_dialog_keys();
}

/// Ctrl/Alt+QWERTY while the goods dialog is open: reprice the stop being edited, in
/// place. The dialog edits the applied route's pool record directly (verified: its +/-
/// buttons change the pool), so we write the level prices into the same record: sells
/// (positive price) get the sell level, buys (negated max price) the buy level.
/// Directions and amounts stay untouched.
pub(crate) unsafe fn reprice_dialog_stop(dialog: u32, stop_index: u32, sell: Option<PriceLevel>, buy: Option<PriceLevel>) {
    let record = *crate::routes::ROUTE_STOP_POOL + stop_index * crate::routes::ROUTE_STOP_SIZE;
    let town = get_town_name(*((record + 2) as *const u8)).unwrap_or_else(|| "<unknown>".into());

    let mut updated = 0;
    for ware_index in crate::ffi::TRADE_WARES {
        // Only slots with a nonzero amount carry an instruction: inactive slots can
        // still hold a (positive) base price the dialog has not normalized yet.
        let amount = *((record + 124 + ware_index as u32 * 4) as *const i32);
        if amount == 0 {
            continue;
        }
        let price = (record + 28 + ware_index as u32 * 4) as *mut i32;
        let new = match *price {
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
        if new != *price {
            *price = new;
            updated += 1;
        }
    }
    if updated > 0 {
        refresh_goods_dialog(dialog, stop_index);
    }
    let what = match (sell, buy) {
        (Some(level), _) => format!("Sell prices {level:?}"),
        (_, Some(level)) => format!("Buy prices {level:?}"),
        _ => "Prices".into(),
    };
    crate::ffi::notify(&format!("{what}: {updated} wares of the {town} stop"));
}

/// F1 while the goods dialog is open: fill the displayed stop's EMPTY ware slots with
/// trade orders for the stop's own town - buy what it produces (at crate::routes::STOP_BUY_LEVEL),
/// sell everything else (at crate::routes::STOP_SELL_LEVEL), MAX amounts - the same shape as a stop of
/// the F4 trade template, but written in place and preserving every existing instruction,
/// office transfers included. With `skip_no_buy_wares` the [crate::routes::NO_BUY_WARES] are not bought
/// where the town produces them - they get a sell order like anything else, not no order.
/// The instruction order is recomputed into the builder's cargo order.
pub(crate) unsafe fn on_dialog_setup_hotkey(dialog: u32, stop_index: u32, skip_no_buy_wares: bool) {
    let record = *crate::routes::ROUTE_STOP_POOL + stop_index * crate::routes::ROUTE_STOP_SIZE;
    let town_index = *((record + 2) as *const u8);
    let town = get_town_name(town_index).unwrap_or_else(|| "<unknown>".into());
    let production = GAME_WORLD_PTR.get_town(town_index).get_production_values();

    let mut price = [0i32; 24];
    let mut amount = [0i32; 24];
    core::ptr::copy_nonoverlapping((record + 28) as *const i32, price.as_mut_ptr(), 24);
    core::ptr::copy_nonoverlapping((record + 124) as *const i32, amount.as_mut_ptr(), 24);

    let mut bought = Vec::new();
    let mut skipped = Vec::new();
    let mut sold = 0;
    let mut untouched = 0;
    for ware_index in crate::ffi::TRADE_WARES {
        let i = ware_index as usize;
        let ware_id = WareId::from_u16(ware_index).unwrap();
        // A slot without an instruction has amount 0 - but NOT necessarily price 0:
        // the game leaves base prices in inactive slots, and the dialog normalizes
        // them to 0 lazily via enqueued operations after opening. Testing the price
        // here would race that queue (observed: quick F1 presses filled fewer wares).
        if amount[i] != 0 {
            untouched += 1;
            continue;
        }
        // Same rule as the F4 trade stop: buy the production minus the crate::routes::NO_BUY_WARES, sell
        // everything else - a produced NO_BUY_WARE included. Kept identical on purpose;
        // the two are documented as the same shape.
        let produced = production[i] > 0;
        if produced && !(skip_no_buy_wares && crate::routes::NO_BUY_WARES.contains(&ware_id)) {
            price[i] = -buy_price(ware_index, crate::routes::STOP_BUY_LEVEL);
            bought.push(format!("{ware_id:?}"));
        } else {
            if produced {
                skipped.push(format!("{ware_id:?}"));
            }
            price[i] = sell_price(ware_index, crate::routes::STOP_SELL_LEVEL);
            sold += 1;
        }
        amount[i] = builder::MAX_AMOUNT;
    }

    core::ptr::copy_nonoverlapping(price.as_ptr(), (record + 28) as *mut i32, 24);
    core::ptr::copy_nonoverlapping(amount.as_ptr(), (record + 124) as *mut i32, 24);
    let order = builder::cargo_order(&amount);
    core::ptr::copy_nonoverlapping(order.as_ptr(), (record + 4) as *mut u8, 24);
    refresh_goods_dialog(dialog, stop_index);

    let skipped = if skipped.is_empty() {
        String::new()
    } else {
        format!(", selling not buying [{}]", skipped.join(", "))
    };
    info!(
        "dialog setup for the {town} stop: buying [{}]{skipped}, selling {sold}, {untouched} existing instructions untouched",
        bought.join(", ")
    );
    crate::ffi::notify(&format!(
        "Stop setup for {town}: {} buys, {sold} sells, {untouched} kept",
        bought.len()
    ));
}
