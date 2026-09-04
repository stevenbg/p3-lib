//! Price levels derived from the game's price curves, shared by the administrator
//! hotkeys and (later) trade route creation.
//!
//! Both curves are continuous piecewise-linear functions of the town's market stock,
//! anchored at the four price thresholds (`TownPtr::get_price_thresholds()`, raw units,
//! recalculated by the game every tick), as factors of the ware's base price:
//!
//! selling: stock 0 -> d, t0 -> 1.4, t1 -> 1.0, t2 -> 0.7, t3+ -> 0.5
//! buying:  stock 0 -> 4.0, t0 -> 1.5, t1 -> 1.0, t2 -> 0.8, t3+ -> 0.6
//!
//! t0 is one week of the town's consumption (building materials use fixed bases and
//! tiny consumptions are clamped to minimum steps, so t0 is not always consumption).
//! A sell limit priced at a stock point makes the administrator stop selling once the
//! market fills up to it; a buy limit makes it drain the market down to it.
//!
//! Because the factors AT the thresholds are constants, a price expressed as a fraction
//! between two thresholds needs no threshold values - it is a fixed multiple of the base
//! price, identical in every town. The live thresholds still decide WHICH STOCK that
//! price corresponds to: when a siege stretches them, the same price leaves the town
//! proportionally more goods. Threshold values are needed for quantities and for
//! reporting what stock a price implies (see [buy_factor_at_stock]).

use num_traits::FromPrimitive;
use p3_api::{data::enums::WareId, town::WARE_BASE_PRICES};

/// The selling curve's factor at an empty market, set by the trade difficulty
/// (2.2 low, 2.0 normal, 1.8 high), read live from the game's settings object.
/// Falls back to normal if the value looks corrupt.
pub unsafe fn difficulty_d() -> f32 {
    let d = *p3_api::TRADE_DIFFICULTY_ADDRESS;
    if (1.0..=3.0).contains(&d) {
        d
    } else {
        2.0
    }
}

/// The six price levels the Q W E R T Y keys set, in ASCENDING price order. Both ladders
/// are given as **absolute factors on the base price**, evenly spaced 0.05 apart, rather
/// than as positions on the price curve - the factor is what the player sets and what
/// decides the margin, so it is the thing worth keeping stable.
///
/// | level | key | BUY | SELL | matched-pair margin |
/// |-|-|-|-|-|
/// | [PriceLevel::Q] | Q | 1.00 | 1.40 | +40.0% |
/// | [PriceLevel::W] | W | 1.05 | 1.45 | +38.1% |
/// | [PriceLevel::E] | E | 1.10 | 1.50 | +36.4% |
/// | [PriceLevel::R] | R | 1.15 | 1.55 | +34.8% |
/// | [PriceLevel::T] | T | 1.20 | 1.60 | +33.3% |
/// | [PriceLevel::Y] | Y | 1.25 | 1.65 | +32.0% |
///
/// Every rung is profitable when bought and sold on the same letter, and buying on a
/// letter at or below the letter you sell on can never lose money.
///
/// Where each lands on the curve, since that is what decides whether a trade fires at
/// all: the buy ladder covers the upper half of the t0..t1 segment (Q is t1 itself, Y is
/// the midpoint), and the sell ladder the upper part of the 0..t0 segment (Q is t0). In
/// weeks of the town's consumption a buy leaves behind:
///
/// | | Q | W | E | R | T | Y |
/// |-|-|-|-|-|-|-|
/// | most wares | 3.0 | 2.8 | 2.6 | 2.4 | 2.2 | 2.0 |
/// | grain (t1 is 5 weeks) | 5.0 | 4.6 | 4.2 | 3.8 | 3.4 | 3.0 |
///
/// Both ladders stay inside their segment for every trade difficulty (`d` is 1.8 to 2.2,
/// and the sell segment spans 1.4 to `d`), so a level always names a reachable stock
/// point. Unlike the previous curve-relative scheme the sell prices no longer move with
/// `d`: the price you set is the price you get, and it is the stock it corresponds to
/// that shifts instead.
#[derive(Clone, Copy, Debug)]
pub enum PriceLevel {
    Q,
    W,
    E,
    R,
    T,
    Y,
}

impl PriceLevel {
    /// (buy factor, sell factor) - the ladder itself.
    const fn factors(self) -> (f32, f32) {
        match self {
            PriceLevel::Q => (1.00, 1.40),
            PriceLevel::W => (1.05, 1.45),
            PriceLevel::E => (1.10, 1.50),
            PriceLevel::R => (1.15, 1.55),
            PriceLevel::T => (1.20, 1.60),
            PriceLevel::Y => (1.25, 1.65),
        }
    }
}

/// The ware's base price per in-game unit (the par value both curves cross at t1).
pub unsafe fn base_price_per_unit(ware_index: u16) -> f32 {
    let scaling = WareId::from_u16(ware_index).unwrap().get_scaling() as f32;
    *WARE_BASE_PRICES.add(ware_index as usize) * scaling
}

/// The buying factor at a level. On the curve this is a point in `t0..t1`, where the
/// buying factor runs 1.5 at t0 down to 1.0 at t1.
pub fn buy_factor(level: PriceLevel) -> f32 {
    level.factors().0
}

/// The selling factor at a level. On the curve this is a point in `0..t0`, where the
/// selling factor runs `d` at an empty market down to 1.4 at t0.
pub fn sell_factor(level: PriceLevel) -> f32 {
    level.factors().1
}

/// The maximum buy price at a level: the administrator drains the market down to that
/// stock point.
pub unsafe fn buy_price(ware_index: u16, level: PriceLevel) -> i32 {
    (buy_factor(level) * base_price_per_unit(ware_index)).round() as i32
}

/// The minimum sell price at a level: the administrator stops selling once the market
/// has filled up to that stock point.
pub unsafe fn sell_price(ware_index: u16, level: PriceLevel) -> i32 {
    (sell_factor(level) * base_price_per_unit(ware_index)).round() as i32
}

/// The buying curve's marginal factor at an arbitrary market stock, interpolating
/// linearly between the anchors (0, 4.0), (t0, 1.5), (t1, 1.0), (t2, 0.8), (t3, 0.6),
/// flat after t3. Degenerate (floored) zero-width segments are skipped. Kept as the
/// reverse-engineered shape of the curve, for pricing stock-denominated targets.
#[allow(dead_code)]
pub fn buy_factor_at_stock(stock: f32, thresholds: &[i32; 4]) -> f32 {
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

/// Note: the game prices a transaction as the average of the curve over the amount
/// traded, not the marginal price at the end point, so a limit set exactly at a level
/// lets the last transaction overshoot that stock point by up to one transaction chunk.
/// Correcting for it needs the game's transaction chunk size, which is not reverse
/// engineered yet; the levels above are deliberately pure marginal factors.
const _: () = ();
