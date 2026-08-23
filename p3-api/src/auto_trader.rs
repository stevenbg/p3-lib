use crate::data::p3_ptr::P3Pointer;

pub const AUTO_TRADER_SIZE: u32 = 0x10;
/// Raw skill points per displayed skill level: level 5 is skill byte 215.
pub const SKILL_PER_LEVEL: u8 = 43;

/// The per-skill ceiling table at `0x00673B34` (a second copy sits at `0x00672824`):
/// 250, 200, 250, 150 raw points, i.e. displayed levels 5, 4, 5, 3.
pub const SKILL_CAPS: [u8; 4] = [250, 200, 250, 150];

/// Age in ticks at which the ten-day captain scan flags a record for retirement
/// (`0x004DCEA9` compares `field_4` against `game_time - 0x474A00`): 18,248 days, almost
/// exactly 50 years.
pub const RETIREMENT_AGE_TICKS: u32 = 0x0047_4A00;

/// The `(navigation, trade, combat)` ceilings for the record at `index`.
///
/// The skill-gain handler `0x00538A80` indexes [SKILL_CAPS] with bits of the record's
/// **array index** - `index & 3` for navigation, `(index >> 2) & 3` for trade,
/// `(index >> 4) & 3` for combat - so the ceiling belongs to the slot, not to the man.
/// Records are recycled through the freelist, so a re-hire inherits whatever the
/// allocator hands out. The handler writes the cap unconditionally when a gain would
/// pass it, which means a skill that starts *above* its cap is pulled **down** to it by
/// the first gain event.
pub fn skill_caps(index: u16) -> (u8, u8, u8) {
    let pick = |shift: u32| SKILL_CAPS[(index >> shift) as usize & 3];
    (pick(0), pick(2), pick(4))
}

/// One record of the auto-trader array: the game's captains, administrators and
/// pirate captains. The array pointer is the first field of the ships container
/// (see `ShipsPtr::get_auto_trader`); towns chain their tavern captain and their
/// pirate captain from `TownPtr::get_auto_trader_chain_head`.
#[derive(Clone, Debug, Copy)]
pub struct AutoTraderPtr {
    pub address: u32,
}

impl AutoTraderPtr {
    pub fn new(address: u32) -> Self {
        Self { address }
    }

    /// The next auto-trader in the same chain (towns chain their auto-traders from
    /// `TownPtr::get_auto_trader_chain_head`); out-of-range ends the chain.
    pub fn get_next_index(&self) -> u16 {
        unsafe { self.get(0x0) }
    }

    /// First/last name ids, rolled by the record initializer 0x4fdf50 modulo the
    /// name-registry counts (0x6DDB70 / 0x6DDB74).
    pub fn get_first_name_id(&self) -> u8 {
        unsafe { self.get(0x2) }
    }

    pub fn get_last_name_id(&self) -> u8 {
        unsafe { self.get(0x3) }
    }

    /// A birth stamp in game ticks: the initializer `0x004FDF50` writes
    /// `game_time - offset` (floored at 0) with `offset = 46720 * (48..79) + 1792 *
    /// (0..31)`, i.e. an age of 24.0 to 40.1 years at creation. The ten-day scan retires
    /// a captain once the age passes [RETIREMENT_AGE_TICKS]; nothing else reads or
    /// rewrites it, so it never moves.
    pub fn get_timestamp(&self) -> u32 {
        unsafe { self.get(0x4) }
    }

    /// The kind byte: the check `0x4FE150` (field_8 > 0x20) discriminates the two
    /// record kinds sharing this array (captains roll `% 11` at creation, pirates
    /// keep the full random byte). For pirates it doubles as the greed byte behind
    /// [Self::get_pirate_loot_share_percent].
    pub fn get_state_byte(&self) -> u8 {
        unsafe { self.get(0x8) }
    }

    /// The loot share a tavern pirate demands: `25 + 5 * ceil(field_8 / 32)`, i.e.
    /// 35%..65% (verified against seven live pirates). Only meaningful for pirate
    /// records.
    pub fn get_pirate_loot_share_percent(&self) -> u32 {
        25 + 5 * ((self.get_state_byte() as u32 + 31) / 32)
    }

    /// True for the records the captain resolver 0x5269A0 accepts (field_8 <= 0x20,
    /// the inverse of check 0x4FE150) - verified against live tavern captains.
    pub fn is_captain(&self) -> bool {
        self.get_state_byte() <= 0x20
    }

    /// True for pirate-captain records (the tavern pirates a ship can be handed to,
    /// one chained per town; identified by matching name ids against the in-game
    /// pirates) - what the sibling resolver 0x5261D0 accepts. Note the kind byte is
    /// a random roll, so the distinction only holds for chained records; parked
    /// administrators live unchained in the same array.
    pub fn is_pirate(&self) -> bool {
        self.get_state_byte() > 0x20
    }

    /// Skill bytes are 43 per displayed level (215 = level 5).
    pub fn get_navigation_skill(&self) -> u8 {
        unsafe { self.get(0x9) }
    }

    /// The trading skill earns the buying discount
    /// `percent_paid = 2 * (50 - skill / 43)` (applied at 0x4d5347 for captains,
    /// 0x4ff7e8/0x4ff944 for administrators).
    pub fn get_trade_skill(&self) -> u8 {
        unsafe { self.get(0xa) }
    }

    pub fn get_combat_skill(&self) -> u8 {
        unsafe { self.get(0xb) }
    }

    /// Set once the captain has been flagged for retirement, by the AI branch of the
    /// ten-day scan (`0x004DCFE0`/`0x004DD023`) or by operation `0x13` (`0x00538CB5`)
    /// for a human owner. Both then schedule task `0x27` to take him off the ship, and
    /// the flag stops the scan from queueing a second removal.
    pub fn get_retirement_flag(&self) -> u8 {
        unsafe { self.get(0xe) }
    }

    /// The 0-5 level the game displays for a raw skill byte.
    pub fn skill_level(skill: u8) -> u8 {
        skill / SKILL_PER_LEVEL
    }

    /// The percentage of a transaction price this auto trader pays when buying:
    /// `2 * (50 - trade_skill / 43)`, so 100% at trade level 0 down to 90% at level
    /// 5 (captains 0x004D5347, administrators 0x004FF7C0 - both buying only).
    pub fn get_buy_percent_paid(&self) -> u32 {
        2 * (50 - Self::skill_level(self.get_trade_skill()) as u32)
    }

    /// The daily wage; caps at 110 in-game.
    pub fn get_daily_wage(&self) -> u16 {
        unsafe { self.get(0xc) }
    }

    /// The employing merchant, `0xFF` = unemployed. Both resolvers prefer a record
    /// employed by the asking merchant and fall back to an unemployed one.
    pub fn get_merchant_index(&self) -> u8 {
        unsafe { self.get(0xf) }
    }
}

impl P3Pointer for AutoTraderPtr {
    fn get_address(&self) -> u32 {
        self.address
    }
}
