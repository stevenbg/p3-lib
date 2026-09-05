//! The church's **feeding the poor** donation: what a gift is worth, and what it buys.
//!
//! Two separate numbers come out of one donation, and they are easy to conflate:
//!
//! - **Reputation** is linear in the delivered value - `value * 0.0003`, credited to the
//!   merchant's social term by the handler `0x004FE557`. No thresholds, no cap.
//! - **The reply**, and with it the one-shot beggar influx, comes from a *gate byte* the
//!   donation dialog computes at `0x005CB0FD`-`0x005CB11C` as
//!   `min(total_value / divisor, 255)`.
//!
//! Both price the goods with the town's own selling-price routine `0x0052E1D0`, which
//! walks the town's price bands against current stock - so the value tracks scarcity.
//! `BASE_PRICE[ware] * 0.5` is only that walk's **floor**, not the valuation: measured in
//! Lübeck, beer came out at 2.7x its base price because the town was short of it.

use crate::data::enums::WareId;
use crate::data::p3_ptr::P3Pointer;
use crate::game_world::GAME_WORLD_PTR;

/// Reputation credited per gold of delivered market value (`0x004FE557`).
///
/// The full expression is `value * 0.0003 / (church_factor + 1) * base_rep_factor`, and
/// both of those are 1.0 and 0.0 respectively in an unmodified game - so a point of
/// reputation costs about 3,333 gold of goods. The credit lands in the **social** term,
/// which decays 1% per game day (the daily merchant recalculation), so donating is a
/// top-up rather than a purchase.
pub const REPUTATION_PER_GOLD: f64 = 0.0003;

/// The gate byte at which the reply becomes "thank you very much for the generous
/// donation" - reputation only, no other effect.
pub const GATE_GENEROUS: i32 = 10;

/// The gate byte at which the reply becomes "An extremely generous donation! Beggars from
/// everywhere will come to the town" - and the handler sets bit `0x800000` of the town
/// flags at `0x004FE85A`, the one-shot **beggar influx** trigger.
///
/// Worth aiming at deliberately: beggars are the town's labour pool, and hiring converts
/// four of them into four poor citizens. A town with an empty pool cannot staff new
/// buildings and falls back to poaching.
pub const GATE_BEGGAR_INFLUX: i32 = 50;

/// The divisor the dialog scales a donation's value by:
/// `trunc(sqrt(citizens * poor_satisfaction / 18)) + 8`.
///
/// A non-positive product never reaches `fsqrt` (`0x0063AB05` branches away on the sign
/// bit), so the term contributes nothing and the divisor floors at 8 - which makes a town
/// with miserable poor by far the cheapest to impress. It also scales with town size, so
/// a capital costs more than a village for the same reply.
pub fn donation_divisor(citizens: i32, poor_satisfaction: i16) -> i32 {
    let product = citizens as i64 * poor_satisfaction as i64 / 18;
    let root = if product > 0 { (product as f64).sqrt() as i64 } else { 0 };
    root as i32 + 8
}

/// The market value, in gold, a donation to `town_index` needs to reach each reply band:
/// `(generous, beggar influx)`. `None` before a game is loaded.
///
/// Compare the figures against what the goods would cost at that town's own prices - the
/// same routine values both - so the cheapest donation is whatever the town is shortest
/// of relative to what you paid for it elsewhere.
pub fn donation_thresholds(town_index: u8) -> Option<(i32, i32)> {
    let town = GAME_WORLD_PTR.get_town(town_index);
    if town.address == 0 {
        return None;
    }
    let divisor = donation_divisor(town.get_citizens(), town.get_poor_satisfaction());
    Some((GATE_GENEROUS * divisor, GATE_BEGGAR_INFLUX * divisor))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lübeck at 3027 citizens and poor satisfaction 10 was measured in game as divisor
    /// 49, reaching the beggar-influx band at exactly 53 barrels of beer.
    #[test]
    fn matches_the_measured_luebeck_case() {
        assert_eq!(donation_divisor(3027, 10), 49);
        assert_eq!(GATE_BEGGAR_INFLUX * 49, 2450);
    }

    /// A non-positive product must not reach the square root; the divisor floors at 8.
    #[test]
    fn unhappy_poor_floor_the_divisor() {
        assert_eq!(donation_divisor(3000, 0), 8);
        assert_eq!(donation_divisor(3000, -20), 8);
    }
}

// ---------------------------------------------------------------------------------------
// The church object itself: `town + 0x794`.
// ---------------------------------------------------------------------------------------

/// The church lives inline in the town at this offset. Every church routine is a method on
/// it - the three operation handlers, the daily tick, and the two getters the window uses.
pub const CHURCH_OFFSET_IN_TOWN: u32 = 0x794;

/// The **church factor**, a global byte at `0x006DE52C`, range `0..4`. Scales every church
/// figure: capacities, extension costs and the reputation divisor are all `(factor + 1)`
/// terms.
///
/// **In single player it is always 0**, so every figure here sits at its base constant.
/// Measured 29 Aug 2026: starting saves on different difficulty levels moved none of the
/// church numbers. The reason is the gate at `0x00547F74` - `test byte [queue],0xc` on the
/// operation queue `0x006DF2F0`, the same flag `0x0054AA70` uses for its networked enqueue
/// path - so operation `0xA9` is built **only in multiplayer**. A single-player game never
/// publishes the setting and the global keeps its BSS zero.
///
/// It comes from the **new-game setup**, traced end to end (but see the gate above - this
/// only happens over the network):
///
/// - operation `0xA9` has one producer, `0x00547F80` (a second packing site at
///   `0x0043DD69`), which reads the game-settings object `[0x006CC3E8]` and puts
///   `settings+0xE` into `op[0xC]`;
/// - its handler `0x00542380` writes `[0x006DE52C] = op[0xC] % 5`;
/// - `0x00502F7F` re-clamps defensively with the same `% 5`, which is what bounds it to
///   `0..4`.
///
/// `settings+0xE` is written by the difficulty preset (`0x00463B17`) and by the Game
/// settings screen (`0x004992F1`, from window field `+0x1BA4`). It is **not** one of the
/// eight labelled difficulty dropdowns: those sit at window `+0x1B80`..`+0x1B98` on a
/// stride of 4 and are all decremented on the way in (the UI positions are 1-based), while
/// `+0x1BA4` is stored raw. **Which control `+0x1BA4` is has not been identified.**
/// `settings+0xE + 1` is also handed to world generation at `0x0054384D`.
///
/// The gitbook's `church_factor = 0.0` recurring constant is therefore right for single
/// player, and right for the reason above rather than by coincidence.
///
/// Everything here reads the live byte rather than assuming zero, so the figures stay right
/// if something does set it.
pub const CHURCH_FACTOR_ADDRESS: *const u8 = 0x006DE52C as _;

/// `(factor + 1)`, the multiplier every capacity below is expressed in.
pub fn church_scale() -> i32 {
    unsafe { *CHURCH_FACTOR_ADDRESS as i32 + 1 }
}

/// Total jewellery-donation money the church will hold, `12000 * (factor + 1)`
/// (`0x004FE30A`-`0x004FE325`).
///
/// **Gold past the cap is taken and wasted.** The handler deducts the whole donation from
/// the merchant at `0x004FE2EE`/`0x004FE2F4`, *before* the cap is even computed; the cap
/// then only shrinks the amount passed to the reputation credit, and the balance is stored
/// as `cap - 1`. So overpaying buys neither decoration nor reputation for the excess - it
/// simply disappears. The same is true of overshooting an extension stage
/// ([extension_cost]): `0x004FE420` takes the money first and clamps afterwards.
pub const JEWELLERY_CAP_PER_SCALE: i32 = 12_000;

/// Gold per step of the 0..5 decoration level, `2000 * (factor + 1)` (`0x004FE360`). The
/// level is `min(round(money / step), 5)`, so it saturates at `4.5 * step` - well below the
/// cap, after which further donations buy reputation and nothing visible.
pub const DECORATION_STEP_PER_SCALE: i32 = 2_000;
/// The highest decoration level, and the only one the window treats differently: at 5 the
/// church interior uses its decorated artwork and animation pair, below it the plain one.
pub const DECORATION_MAX: i32 = 5;

/// Jewellery money **decays 100 gold per day** in the church's daily tick
/// (`0x004FE885`: `if money >= 100 { money -= 100 } else { money = 0 }`), so the decoration
/// level slides back down unless it is topped up. The extension fund at `+0x4` does **not**
/// decay.
pub const JEWELLERY_DECAY_PER_DAY: i32 = 100;

/// Gold an extension stage needs: `BASE[stage] + (factor) * MULT[stage]`, from
/// `0x004FE420` and the base table at `0x006734B4`. `None` past the last stage.
pub fn extension_cost(stage: u8) -> Option<i32> {
    const BASE: [i32; 3] = [20_000, 40_000, 60_000];
    const MULT: [i32; 3] = [10_000, 15_000, 20_000];
    let stage = stage as usize;
    if stage >= BASE.len() {
        return None;
    }
    let factor = church_scale() - 1;
    Some(BASE[stage] + factor * MULT[stage])
}

/// The stage at which the church is fully extended and takes no more extension money.
pub const EXTENSION_STAGES: u8 = 3;

/// The goods an extension consumes, and how many units of each per stage - ware ids from
/// `0x006734BC`, amounts from `0x006734C0` (stride 4, indexed by stage).
///
/// The daily tick (`0x004FE877`) only builds once the fund is full **and** all three are
/// stocked; it draws them from the **town's own stock**, taking only what sits above half
/// the ware's t0 price threshold, so a town short of bricks stalls a fully funded
/// extension.
pub const EXTENSION_MATERIALS: [(WareId, [u8; 3]); 3] = [
    (WareId::Timber, [20, 20, 30]),
    (WareId::IronGoods, [20, 20, 30]),
    (WareId::Bricks, [50, 50, 60]),
];

/// A town's church.
#[derive(Clone, Debug, Copy)]
pub struct ChurchPtr {
    pub address: u32,
}

impl ChurchPtr {
    /// `None` before a game is loaded.
    pub fn of_town(town_index: u8) -> Option<Self> {
        let town = GAME_WORLD_PTR.get_town(town_index);
        if town.address == 0 {
            return None;
        }
        Some(Self { address: town.address + CHURCH_OFFSET_IN_TOWN })
    }

    /// Jewellery-donation money (`+0x0`), capped at
    /// [JEWELLERY_CAP_PER_SCALE] `* (factor + 1)` and decaying daily.
    pub fn get_jewellery_money(&self) -> i32 {
        unsafe { self.get(0x0) }
    }

    /// Money collected toward the current extension stage (`+0x4`). Clamped to the stage's
    /// [extension_cost] as it is paid in; does not decay.
    pub fn get_extension_money(&self) -> i32 {
        unsafe { self.get(0x4) }
    }

    /// A counter the daily tick increments (`+0x8`), initialised to `255` by
    /// `0x004FE2B0`. Extension donations are **refused** unless it exceeds 100
    /// (`cmp [eax+8],0x64 / ja` at `0x004FE4C3`), which is also what makes the window say
    /// the church is not planning a further extension.
    pub fn get_extension_offer_counter(&self) -> i32 {
        unsafe { self.get(0x8) }
    }

    /// Units of each [EXTENSION_MATERIALS] ware the church has gathered (`+0xC`..`+0xE`).
    pub fn get_extension_materials(&self) -> [u8; 3] {
        unsafe { [self.get(0xc), self.get(0xd), self.get(0xe)] }
    }

    /// The extension stage, `0`..[EXTENSION_STAGES] (`+0xF`). At [EXTENSION_STAGES] the
    /// church is fully extended.
    pub fn get_extension_stage(&self) -> u8 {
        unsafe { self.get(0xf) }
    }

    /// The 0..[DECORATION_MAX] decoration level the window renders, as `0x004FE360`
    /// computes it: `min(round(money / step), 5)`.
    pub fn decoration_level(&self) -> i32 {
        let step = DECORATION_STEP_PER_SCALE * church_scale();
        ((self.get_jewellery_money() + step / 2) / step).min(DECORATION_MAX)
    }

    /// Whether extension donations are currently accepted: not already fully extended, not
    /// already funded, and the offer counter above 100.
    pub fn accepts_extension_money(&self) -> bool {
        let stage = self.get_extension_stage();
        stage < EXTENSION_STAGES
            && extension_cost(stage).is_some_and(|cost| self.get_extension_money() < cost)
            && self.get_extension_offer_counter() > 100
    }
}

impl P3Pointer for ChurchPtr {
    fn get_address(&self) -> u32 {
        self.address
    }
}
