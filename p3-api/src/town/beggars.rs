//! The beggar pool: the town's **labour intake**.
//!
//! Beggars sit outside the jobs identity and have their own equilibrium, maintained by
//! `0x0051C0E0` once per town tick. They matter because hiring converts **four beggars into
//! four poor citizens** (`0x005108E0`): a town with an empty pool cannot staff new
//! buildings and falls back to poaching from existing ones.
//!
//! Every figure here is integer arithmetic lifted from the routine, truncation included -
//! a float model gets the target wrong by one in a good fraction of towns.

/// The floor the target never goes below, and the whole target when the beggars are
/// unhappy (`mov edi,8` at `0x0051C0E8`).
pub const BEGGAR_TARGET_FLOOR: i32 = 8;

/// The equilibrium the pool drifts toward: `sqrt(citizens * satisfaction / 2) / 3 + 8`,
/// or just the floor when beggar satisfaction is `<= 0` (`test ax,ax / jle` at
/// `0x0051C0FA`).
///
/// Every division truncates and the square root is taken of an already-truncated product,
/// which is what makes this reproduce the game exactly.
///
/// Beggar satisfaction is never lowered, and repelling a siege raises it - so a town that
/// fights off attackers keeps a permanently higher intake.
pub fn beggar_target(citizens: i32, beggar_satisfaction: i16) -> i32 {
    if beggar_satisfaction <= 0 {
        return BEGGAR_TARGET_FLOOR;
    }
    let product = (citizens as i64 * beggar_satisfaction as i64) >> 1;
    let root = (product.max(0) as f64).sqrt() as i64;
    (root / 3) as i32 + BEGGAR_TARGET_FLOOR
}

/// How many beggars the one-shot influx adds, the jump a large feed-the-poor donation
/// buys (`0x0051C243`-`0x0051C27F`).
///
/// `sqrt(citizens) / 6 + target / 2`, then capped so the pool stays under a quarter of the
/// population - but **only once the town already holds more than 50** (`cmp ecx,0x32 / jle`
/// at `0x0051C25B`). Below that the influx is unbounded, which is what makes it worth so
/// much to a small or freshly drained town.
///
/// The routine also requires town flag `0x8` to be clear: it tests both bits at once
/// (`and edx,0x800008 / cmp edx,0x800000`), and `0x8` is the same flag that blocks ordinary
/// beggar growth. A town carrying it gets nothing and the trigger bit is not consumed.
pub fn beggar_influx(citizens: i32, beggars: i32, target: i32) -> i32 {
    let root = (citizens.max(0) as f64).sqrt() as i32;
    let delta = root / 6 + target / 2;
    if beggars <= 50 {
        return delta;
    }
    if (beggars + delta) * 4 <= citizens {
        return delta;
    }
    (citizens / 4 - beggars).max(0)
}

/// How fast the pool drains when it sits **above** [beggar_target]:
/// `(sqrt(citizens) + 9) / 10` per town tick, i.e. per day.
///
/// Beggars arrive at twice the rate they leave - the increase is
/// `(sqrt(citizens) + 4) / 5` - but an influx overshoots the target, and above it only
/// this branch runs. So the jump a donation buys is a **window**, not a gain: what is not
/// hired within it wanders off.
pub fn beggar_decay_per_day(citizens: i32) -> i32 {
    let root = (citizens.max(0) as f64).sqrt() as i32;
    ((root + 9) / 10).max(1)
}

/// Roughly how many days an influx survives before the pool is back at its target, or
/// `None` when the influx does not overshoot - in which case nothing decays, it simply
/// arrives at the equilibrium sooner.
pub fn beggar_influx_lifetime_days(citizens: i32, beggars: i32, target: i32, influx: i32) -> Option<i32> {
    let surplus = beggars + influx - target;
    if surplus <= 0 {
        return None;
    }
    Some((surplus + beggar_decay_per_day(citizens) - 1) / beggar_decay_per_day(citizens))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Three towns from a live save, read at beggar satisfaction 60, held exactly these
    /// pools. Any rounding mistake shows up here.
    #[test]
    fn reproduces_the_measured_targets() {
        assert_eq!(beggar_target(5329, 60), 141);
        assert_eq!(beggar_target(4149, 60), 125);
        assert_eq!(beggar_target(3942, 60), 122);
    }

    /// Every town at satisfaction -20 held exactly 8.
    #[test]
    fn unhappy_beggars_sit_at_the_floor() {
        assert_eq!(beggar_target(5329, -20), BEGGAR_TARGET_FLOOR);
        assert_eq!(beggar_target(5329, 0), BEGGAR_TARGET_FLOOR);
    }

    /// A 3000-citizen town at target 141 holding 104 beggars: the influx overshoots by 42,
    /// which drains at 6 a day.
    #[test]
    fn an_overshooting_influx_is_a_window() {
        let influx = beggar_influx(3000, 104, 141);
        assert_eq!(influx, 54 / 6 + 70);
        assert_eq!(beggar_decay_per_day(3000), 6);
        assert_eq!(beggar_influx_lifetime_days(3000, 104, 141, influx), Some(7));
    }

    /// Below the target nothing decays - the influx just arrives at the equilibrium sooner.
    #[test]
    fn an_influx_that_does_not_overshoot_keeps() {
        assert_eq!(beggar_influx_lifetime_days(3000, 10, 141, 20), None);
    }

    /// Under 50 beggars the influx is unbounded; over it, the quarter-of-population cap
    /// applies and can clamp the jump to nothing.
    #[test]
    fn the_cap_only_applies_above_fifty() {
        // 3000 citizens, target 141, 20 beggars: no cap, the full jump lands.
        assert_eq!(beggar_influx(3000, 20, 141), 54 / 6 + 70);
        // Already at a quarter of the population: nothing to give.
        assert_eq!(beggar_influx(3000, 750, 141), 0);
    }
}
