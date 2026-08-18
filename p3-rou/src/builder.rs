//! Building trade route stops, and the format semantics this encodes (all verified
//! against game-saved routes):
//!
//! - Per stop and ware, the operation is encoded by the signs of price and amount:
//!   load office -> ship = price 0, positive amount; unload ship -> office = price 0,
//!   negative amount; sell to town = positive minimum price, positive amount; buy from
//!   town = negated maximum price, positive amount.
//! - Amounts are raw units: in-game units times the ware scaling (loads 2000,
//!   barrels 200, weapons 10). [MAX_AMOUNT] means "as much as possible", unscaled.
//! - The action byte carries the repair flag ([FLAG_R]/[FLAG_X]/[FLAG_NONE]) and
//!   [FIRST_STOP_MARKER] on the route's first stop.
//! - The per-stop order array is the instruction order; [partitioned_order] puts
//!   unloading wares first so the ship frees space before taking new cargo on.

use std::str::FromStr;

use num_traits::FromPrimitive;
use p3_api::data::enums::WareId;

use crate::TradeRouteStop;

/// The game's sentinel for "as much as possible", stored unscaled.
pub const MAX_AMOUNT: i32 = 1_000_000_000;
/// Raw units per in-game unit: wares measured in loads (1 load = 10 barrels).
pub const LOAD_SCALING: i32 = 2000;
/// Raw units per in-game unit: wares measured in barrels.
pub const BARREL_SCALING: i32 = 200;
/// Set on the first stop of every route the game saves.
pub const FIRST_STOP_MARKER: u8 = 0x04;
/// Repair the ship at this stop.
pub const FLAG_R: u8 = 0x01;
pub const FLAG_X: u8 = 0x00;
pub const FLAG_NONE: u8 = 0x09;

/// The ware display order the game writes into every saved route.
pub const DEFAULT_ORDER: [u8; 24] = [
    0x03, 0x13, 0x08, 0x02, 0x00, 0x11, 0x05, 0x0c, 0x0d, 0x01, 0x10, 0x0f, 0x12, 0x04, 0x09, 0x06, 0x0b, 0x0a, 0x07, 0x0e, 0x15, 0x17, 0x16, 0x14,
];

/// Looks up a ware by its exact `WareId` identifier (e.g. "PigIron").
pub fn ware_index(name: &str) -> Option<usize> {
    WareId::from_str(name).ok().map(|ware| ware as usize)
}

/// The ware's raw-unit scaling (raw units per in-game unit), from p3-api.
pub fn ware_scaling(index: usize) -> i32 {
    WareId::from_usize(index).map(|ware| ware.get_scaling()).unwrap_or(1)
}

/// A stop with its instructions in [cargo_order]. Construct [TradeRouteStop] directly to
/// keep the game's default display order instead.
pub fn stop(town_index: u8, action: u8, price: [i32; 24], amount: [i32; 24]) -> TradeRouteStop {
    TradeRouteStop {
        town_index,
        action,
        order: cargo_order(&price, &amount),
        price,
        amount,
    }
}

/// The trade wares from worst to best value per unit of hold space - a trading
/// judgment, used to decide what fills the hold first.
pub const VALUE_ORDER_WORST_TO_BEST: [WareId; 20] = [
    WareId::Timber,
    WareId::Bricks,
    WareId::Grain,
    WareId::Salt,
    WareId::Beer,
    WareId::Hemp,
    WareId::Fish,
    WareId::Pitch,
    WareId::WhaleOil,
    WareId::Wool,
    WareId::PigIron,
    WareId::Meat,
    WareId::Honey,
    WareId::Pottery,
    WareId::Cloth,
    WareId::Wine,
    WareId::Leather,
    WareId::IronGoods,
    WareId::Spices,
    WareId::Skins,
];

/// 0 for the best-valued trade ware, 19 for the worst; the weapons (which never carry
/// route orders) rank after everything.
fn value_rank(ware: u8) -> u8 {
    VALUE_ORDER_WORST_TO_BEST
        .iter()
        .position(|&w| w as u8 == ware)
        .map(|pos| (VALUE_ORDER_WORST_TO_BEST.len() - 1 - pos) as u8)
        .unwrap_or(VALUE_ORDER_WORST_TO_BEST.len() as u8)
}

/// The instruction order for generated routes: everything that frees cargo space runs
/// before anything that fills it, and the filling instructions take the barrel goods
/// before the bulky loads goods (1 load = 10 barrels of hold space), each group ordered
/// best value first - so when hold space runs out, the least valuable cargo is what
/// gets left behind.
///
/// Freeing space: selling to the town (positive price) and unloading into the office
/// (negative amount). Filling it: buying from the town (negative price) and loading from
/// the office (zero price, positive amount).
pub fn cargo_order(price: &[i32; 24], amount: &[i32; 24]) -> [u8; 24] {
    ordered_by_key(|ware| {
        let i = ware as usize;
        if amount[i] == 0 {
            return u8::MAX; // no instruction for this ware
        }
        if amount[i] < 0 || price[i] > 0 {
            return 0; // unload into the office, or sell to the town
        }
        // Buy from the town or load from the office. Barrel/loads grouping is keyed off
        // the scaling because WareId::is_barrel_ware panics on the weapons, which occupy
        // slots in the order array even though they never carry route orders.
        let group = if ware_scaling(i) == BARREL_SCALING { 32 } else { 64 };
        group + value_rank(ware)
    })
}

/// The order array is the stop's instruction order: wares are grouped by `key`, groups
/// in ascending order, each group keeping the game's default display order.
pub fn ordered_by_key(key: impl Fn(u8) -> u8) -> [u8; 24] {
    let mut order = DEFAULT_ORDER;
    // A stable sort keeps the default display order within each group.
    order.sort_by_key(|&ware| key(ware));
    order
}

/// The wares for which `first` holds are listed before the rest.
pub fn ordered_by(first: impl Fn(u8) -> bool) -> [u8; 24] {
    ordered_by_key(|ware| if first(ware) { 0 } else { 1 })
}

/// Sell without an amount limit at the given minimum prices, and take back exactly the
/// given amounts, for every ware with a nonzero load amount.
fn sell_max_and_unload(load_amount: &[i32; 24]) -> ([i32; 24], [i32; 24]) {
    let mut sell_max = [0i32; 24];
    let mut unload_calculated = [0i32; 24];
    for (i, load) in load_amount.iter().enumerate() {
        if *load != 0 {
            sell_max[i] = MAX_AMOUNT;
            unload_calculated[i] = -load;
        }
    }
    (sell_max, unload_calculated)
}

/// Supply route: load the given raw amounts at the source (repairing there), sell max
/// at the given minimum prices in the target town, reset the target office's stock of
/// the supplied wares to exactly the loaded amounts (take max of just those, put the
/// amounts back) while collecting every other ware from that office, and unload the
/// whole ship into the source office.
///
/// Taking only the supplied wares at the third stop bounds the hold space the reset
/// needs; the collected wares ride along from the fourth stop, where the put-back has
/// just freed space (unloads run before loads within a stop, see [cargo_order]).
pub fn five_stop_route(load_town: u8, sell_town: u8, load_amount: [i32; 24], sell_prices: [i32; 24]) -> Vec<TradeRouteStop> {
    let (sell_max, unload_calculated) = sell_max_and_unload(&load_amount);
    let mut take_supplied = [0i32; 24];
    let mut put_back_and_collect = unload_calculated;
    for i in 0..24 {
        if load_amount[i] != 0 {
            take_supplied[i] = MAX_AMOUNT;
        } else {
            put_back_and_collect[i] = MAX_AMOUNT;
        }
    }
    vec![
        stop(load_town, FLAG_R | FIRST_STOP_MARKER, [0i32; 24], load_amount),
        stop(sell_town, FLAG_X, sell_prices, sell_max),
        stop(sell_town, FLAG_X, [0i32; 24], take_supplied),
        stop(sell_town, FLAG_X, [0i32; 24], put_back_and_collect),
        stop(load_town, FLAG_X, [0i32; 24], [-MAX_AMOUNT; 24]),
    ]
}

/// Like [five_stop_route], but swaps the target office's stock of the supplied wares
/// one unit category at a time (unload the loads goods + take the supplied barrels,
/// then put back the barrel amounts + take the supplied loads goods, then put back the
/// loads amounts + collect every ware the route does not supply), which bounds the ship
/// space the shuffle needs.
pub fn six_stop_route(load_town: u8, sell_town: u8, load_amount: [i32; 24], sell_prices: [i32; 24]) -> Vec<TradeRouteStop> {
    let (sell_max, unload_calculated) = sell_max_and_unload(&load_amount);
    let mut unload_loads_take_barrels = [0i32; 24];
    let mut unload_barrels_take_loads = [0i32; 24];
    let mut put_back_loads_and_collect = [0i32; 24];
    for i in 0..24 {
        let supplied = load_amount[i] != 0;
        match ware_scaling(i) {
            LOAD_SCALING => {
                unload_loads_take_barrels[i] = -MAX_AMOUNT;
                if supplied {
                    unload_barrels_take_loads[i] = MAX_AMOUNT;
                    put_back_loads_and_collect[i] = unload_calculated[i];
                }
            }
            BARREL_SCALING if supplied => {
                unload_loads_take_barrels[i] = MAX_AMOUNT;
                unload_barrels_take_loads[i] = unload_calculated[i];
            }
            _ => {}
        }
        if !supplied {
            put_back_loads_and_collect[i] = MAX_AMOUNT;
        }
    }
    // The unloads-before-loads ordering of the swap stops comes from [cargo_order].
    vec![
        stop(load_town, FLAG_R | FIRST_STOP_MARKER, [0i32; 24], load_amount),
        stop(sell_town, FLAG_X, sell_prices, sell_max),
        stop(sell_town, FLAG_X, [0i32; 24], unload_loads_take_barrels),
        stop(sell_town, FLAG_X, [0i32; 24], unload_barrels_take_loads),
        stop(sell_town, FLAG_X, [0i32; 24], put_back_loads_and_collect),
        stop(load_town, FLAG_X, [0i32; 24], [-MAX_AMOUNT; 24]),
    ]
}

/// Collection route parked in one town: unload everything into the office (with the
/// repair flag), then five stops buying max of every ware with a positive maximum
/// price in `buy_prices`.
pub fn suck_route(town: u8, buy_prices: [i32; 24]) -> Vec<TradeRouteStop> {
    let mut prices = [0i32; 24];
    let mut buy_max = [0i32; 24];
    for (i, &price) in buy_prices.iter().enumerate() {
        if price > 0 {
            prices[i] = -price;
            buy_max[i] = MAX_AMOUNT;
        }
    }
    let mut stops = vec![stop(town, FLAG_R | FIRST_STOP_MARKER, [0i32; 24], [-MAX_AMOUNT; 24])];
    for _ in 0..5 {
        stops.push(stop(town, FLAG_X, prices, buy_max));
    }
    stops
}
