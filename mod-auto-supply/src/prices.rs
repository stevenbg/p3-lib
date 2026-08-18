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

/// Which segment of a curve a level sits on: below or above the curve's central
/// threshold (BUY: t1, SELL: t0).
#[derive(Clone, Copy, Debug)]
enum Segment {
    Lower,
    Upper,
}

/// Six positions on a price curve, shared by both directions. The first three lie on
/// the segment below the curve's central threshold, the last three above it:
///
/// | level | BUY (t0 -> t1 -> t2) | SELL (0 -> t0 -> t1) |
/// |-------|----------------------|----------------------|
/// | LowerMid | mid t0..t1 (1.25) | mid 0..t0 ((d+1.4)/2) |
/// | Lower70  | 70% t0->t1 (1.15) | 70% 0->t0 |
/// | Center   | t1 (1.0, par)     | t0 (1.4, the supply price) |
/// | Upper30  | 30% t1->t2 (0.94) | 30% t0->t1 (1.28) |
/// | UpperMid | mid t1..t2 (0.90) | mid t0..t1 (1.20) |
/// | Upper    | t2 (0.80)         | t1 (1.0) |
#[derive(Clone, Copy, Debug)]
pub enum PriceLevel {
    LowerMid,
    Lower70,
    Center,
    Upper30,
    UpperMid,
    Upper,
}

impl PriceLevel {
    /// The level as a segment and a fraction along it.
    fn position(self) -> (Segment, f32) {
        match self {
            PriceLevel::LowerMid => (Segment::Lower, 0.5),
            PriceLevel::Lower70 => (Segment::Lower, 0.7),
            PriceLevel::Center => (Segment::Lower, 1.0),
            PriceLevel::Upper30 => (Segment::Upper, 0.3),
            PriceLevel::UpperMid => (Segment::Upper, 0.5),
            PriceLevel::Upper => (Segment::Upper, 1.0),
        }
    }
}

/// The ware's base price per in-game unit (the par value both curves cross at t1).
pub unsafe fn base_price_per_unit(ware_index: u16) -> f32 {
    let scaling = WareId::from_u16(ware_index).unwrap().get_scaling() as f32;
    *WARE_BASE_PRICES.add(ware_index as usize) * scaling
}

/// The buying curve's factor at a level: 1.5 at t0 -> 1.0 at t1 -> 0.8 at t2.
pub fn buy_factor(level: PriceLevel) -> f32 {
    match level.position() {
        (Segment::Lower, f) => 1.5 - 0.5 * f,
        (Segment::Upper, f) => 1.0 - 0.2 * f,
    }
}

/// The selling curve's factor at a level: d at an empty market -> 1.4 at t0 -> 1.0 at t1.
pub unsafe fn sell_factor(level: PriceLevel) -> f32 {
    let d = difficulty_d();
    match level.position() {
        (Segment::Lower, f) => d - (d - 1.4) * f,
        (Segment::Upper, f) => 1.4 - 0.4 * f,
    }
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
