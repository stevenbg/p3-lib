use crate::data::p3_ptr::P3Pointer;

pub const AUTO_TRADER_SIZE: u32 = 0x10;
/// Raw skill points per displayed skill level: level 5 is skill byte 215.
pub const SKILL_PER_LEVEL: u8 = 43;

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

    /// Stamped from the current date serial at creation (0x4fdf50: date minus a
    /// random offset, floored at 0); exact semantics still open.
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
