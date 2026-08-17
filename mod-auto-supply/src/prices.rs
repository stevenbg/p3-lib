//! Price levels computed from the game's live data, shared by the administrator
//! hotkeys and, later, trade route creation.
//!
//! Both price curves are continuous piecewise-linear functions of the town's market
//! stock, anchored at the four price thresholds (`TownPtr::get_price_thresholds()`,
//! raw units, recalculated by the game every tick):
//!
//! selling: stock 0 -> d, t0 -> 1.4, t1 -> 1.0, t2 -> 0.7, t3+ -> 0.5 (x base price)
//! buying:  stock 0 -> 4.0, t0 -> 1.5, t1 -> 1.0, t2 -> 0.8, t3+ -> 0.6
//!
//! t0 is one week of the town's consumption. A buy limit priced at a stock point makes
//! the administrator drain the market down to that point; a sell limit stops selling
//! once the market fills up to it. Midpoint levels interpolate linearly, so they are
//! fixed factors independent of the actual threshold values.

use num_traits::FromPrimitive;
use p3_api::{data::enums::WareId, town::WARE_BASE_PRICES};

/// The selling curve's factor at stock 0, read live from the game's settings object
/// (2.2 low, 2.0 normal, 1.8 high). Falls back to normal if the value looks corrupt.
/// Affects [SellLevel::MidZeroT0] and the chunk correction of [SellLevel::AtT0].
pub unsafe fn difficulty_d() -> f32 {
    let d = *p3_api::TRADE_DIFFICULTY_ADDRESS;
    if (1.0..=3.0).contains(&d) {
        d
    } else {
        2.0
    }
}

#[derive(Clone, Copy, Debug)]
pub enum SellLevel {
    /// The supply price: selling stops once the town holds a week of consumption.
    AtT0,
    /// Halfway between an empty market and t0.
    MidZeroT0,
    /// Halfway between t0 and t1.
    MidT0T1,
    /// Selling stops at t1.
    AtT1,
}

#[derive(Clone, Copy, Debug)]
pub enum BuyLevel {
    /// Halfway between t0 and t1: drains the market down to well below two weeks.
    MidT0T1,
    /// Drains the market down to two weeks of consumption (stock 2 x t0). Coincides
    /// with MidT0T1 for normal wares (t1 = 3 x t0), but differs where t1 is special
    /// (grain bonus, minimum floors, building materials).
    TwoWeeksSupply,
    /// Drains the market down to t1.
    AtT1,
    /// Halfway between t1 and t2.
    MidT1T2,
    /// Drains the market down to t2 - only surplus above that is bought.
    AtT2,
}

/// The buying curve's marginal factor at an arbitrary market stock, interpolating
/// linearly between the anchors (0, 4.0), (t0, 1.5), (t1, 1.0), (t2, 0.8), (t3, 0.6),
/// flat after t3. Degenerate (floored) zero-width segments are skipped.
fn buy_factor_at_stock(stock: f32, thresholds: &[i32; 4]) -> f32 {
    let [t0, t1, t2, t3] = thresholds.map(|t| t as f32);
    let interp = |x0: f32, x1: f32, f0: f32, f1: f32| f0 + (f1 - f0) * ((stock - x0) / (x1 - x0));
    if stock <= t0 && t0 > 0.0 {
        interp(0.0, t0, 4.0, 1.5)
    } else if stock <= t1 && t1 > t0 {
        interp(t0, t1, 1.5, 1.0)
    } else if stock <= t2 && t2 > t1 {
        interp(t1, t2, 1.0, 0.8)
    } else if stock <= t3 && t3 > t2 {
        interp(t2, t3, 0.8, 0.6)
    } else {
        0.6
    }
}

pub unsafe fn base_price_per_unit(ware_index: u16) -> f32 {
    let scaling = WareId::from_u16(ware_index).unwrap().get_scaling() as f32;
    *WARE_BASE_PRICES.add(ware_index as usize) * scaling
}

pub unsafe fn sell_price(ware_index: u16, thresholds: &[[i32; 4]; 24], level: SellLevel) -> i32 {
    let factor = match level {
        SellLevel::AtT0 => {
            // 1.4 at the anchor, plus the averaging over the last traded unit (the
            // segment below t0 runs from the difficulty factor down to 1.4 over t0 raw units),
            // which matters for low-volume wares.
            let scaling = WareId::from_u16(ware_index).unwrap().get_scaling() as f32;
            let t0 = thresholds[ware_index as usize][0] as f32;
            if t0 > 0.0 {
                1.4 + ((difficulty_d() - 1.4) / 2.0) * (scaling / t0)
            } else {
                1.4
            }
        }
        SellLevel::MidZeroT0 => (difficulty_d() + 1.4) / 2.0,
        SellLevel::MidT0T1 => 1.2,
        SellLevel::AtT1 => 1.0,
    };
    (factor * base_price_per_unit(ware_index)).round() as i32
}

pub unsafe fn buy_price(ware_index: u16, thresholds: &[[i32; 4]; 24], level: BuyLevel) -> i32 {
    let t = &thresholds[ware_index as usize];
    let factor = match level {
        BuyLevel::MidT0T1 => 1.25,
        BuyLevel::TwoWeeksSupply => buy_factor_at_stock(2.0 * t[0] as f32, t),
        BuyLevel::AtT1 => 1.0,
        BuyLevel::MidT1T2 => 0.9,
        BuyLevel::AtT2 => 0.8,
    };
    (factor * base_price_per_unit(ware_index)).round() as i32
}
