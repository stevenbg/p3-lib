use std::mem;

use crate::{data::p3_ptr::P3Pointer, latin1_ptr_to_string};

pub const MERCHANT_SIZE: u32 = 0x650;

/// The per-town rank byte ([MerchantPtr::get_rank_in]) at which a merchant may stand for
/// mayor and be appointed by operation 0x46 (`cmp ..,5` at `0x00528D79` / `0x0053602F`).
pub const RANK_PATRICIAN: u8 = 5;
/// Written to a town notable's rank byte while he holds the mayor's seat (`0x00529149`).
pub const RANK_MAYOR: u8 = 6;
/// Written to the alderman's rank bytes (`0x004F7653`) and to a notable mayor who is the
/// alderman (`0x0052913F`).
pub const RANK_ALDERMAN: u8 = 7;

#[derive(Clone, Debug)]
pub struct MerchantPtr {
    pub address: u32,
}

impl MerchantPtr {
    pub fn new(address: u32) -> Self {
        Self { address }
    }

    pub fn get_money(&self) -> i32 {
        unsafe { self.get(0x0) }
    }

    /// The control word. **0 is a human player**, non-zero an AI merchant (game setup
    /// writes `0x8001`); bit `0x4` marks the AI merchants pirates leave alone. It also
    /// picks which growth path the ten-day scan gives this merchant's captains.
    pub fn get_control_word(&self) -> u16 {
        unsafe { self.get(0x8) }
    }

    pub fn get_company_value(&self) -> i32 {
        unsafe { self.get(0x46c) }
    }

    /// Given name and family name, both heap `char*` in the game's latin1 codepage.
    pub unsafe fn get_name(&self) -> String {
        latin1_ptr_to_string(self.get::<u32>(0xe8) as *const u8)
    }

    pub unsafe fn get_family_name(&self) -> String {
        latin1_ptr_to_string(self.get::<u32>(0xe4) as *const u8)
    }

    /// The merchant's home town, as shown on the Personal screen: the town holding the
    /// home office, which changes when the player moves it. Not the town the merchant
    /// was born in, which the same screen lists separately.
    pub fn get_hometown_index(&self) -> u8 {
        unsafe { self.get(0x19) }
    }

    /// Fleet statistics, recomputed by `update_merchant_reputation_and_value`
    /// (`0x004F7BB0`, ship loop at `0x004F7DB5`): total capacity of the merchant's ships.
    pub fn get_fleet_capacity(&self) -> i32 {
        unsafe { self.get(0x470) }
    }

    /// Total **crew** across the merchant's ships (`ship+0x40` summed at `0x004F7E27`).
    /// This is the metric the quarterly task `0x2C` governor watches.
    pub fn get_fleet_crew(&self) -> i32 {
        unsafe { self.get(0x474) }
    }

    /// Ships not in state `0x11`/`0x0E` (the loop's skip states at `0x004F7DDD`).
    pub fn get_active_ship_count(&self) -> u16 {
        unsafe { self.get(0x478) }
    }

    /// Every ship of the merchant, active or not.
    pub fn get_total_ship_count(&self) -> i32 {
        unsafe { self.get(0x47c) }
    }

    /// This merchant's rank **in one town**, as a byte at `+0x39C + town_index`. It is
    /// recomputed next to `update_merchant_reputation_and_value` from the per-town
    /// reputation float at `+0x2FC + town*4` and the company value at `+0x46C` (the
    /// `0xDBBA0` = 900,000 compare at `0x004F7AD4` is the Patrician step). Observed 3..5
    /// for AI merchants in a live 24-town game. The top of the scale is pinned by the
    /// mayor code ([RANK_PATRICIAN], [RANK_MAYOR], [RANK_ALDERMAN]); below that treat it as
    /// an ordinal.
    ///
    /// The pirate AI reads the **home town** entry as its "is this merchant worth robbing"
    /// test - see [crate::game_setup::pirate_attack_rank_threshold].
    pub fn get_rank_in(&self, town_index: u8) -> u8 {
        unsafe { self.get(0x39c + town_index as u32) }
    }

    /// The merchant's reputation **in one town**: the float at `+0x2FC + town*4` that
    /// `update_merchant_reputation_and_value` (`0x004F7BB0`) assembles. The mayor election
    /// scores its candidates by this value truncated to an integer (`0x004F9580`).
    ///
    /// Most terms are **local to the town** they are earned in:
    /// - `+1.0` in the town whose outrigger ship this merchant owns (`0x004F7F87`);
    /// - per office in [Self::get_first_office_index]'s chain (next at `office+0x2C8`),
    ///   credited to the office's town (`office+0x2C6`): the residents of his houses there
    ///   × the rent factor × `0.003` (`0x004F8002`..`0x004F8032`) and the employees of his
    ///   businesses there (`record+0x4`, chained from `office+0x2CC`) × `0.01`
    ///   (`0x004F8083`);
    /// - the three per-town slots at `+0x11C` (buildings, social, trading; stride `0xC`).
    ///
    /// Merchant-wide, added to every town (`0x004F817F`): `min(5, capacity / 100000) +
    /// min(5, company_value / 100000)`, times the base factor at `+0x464`. The spouse bonus
    /// (`+0x32`) goes to the hometown only (`0x004F81B1`). Social and trading are the only
    /// slots multiplied by `0.99` per daily update (`0x004F81DC`).
    pub fn get_reputation_in(&self, town_index: u8) -> f32 {
        unsafe { self.get(0x2fc + town_index as u32 * 4) }
    }

    /// The mayor-candidature flag at `+0x118` (`0x004F9360`): a human's is set by the
    /// constructor (`0x004F72D3`) and toggled by [crate::operation::Operation::SetCandidature];
    /// an AI merchant always stands unless its control word has bit `0x4`.
    pub fn is_mayor_candidate(&self) -> bool {
        unsafe { self.get::<i32>(0x118) != 0 }
    }

    /// Whether the merchant is a guild member in `town_index`: bit `town` of the bitmap at
    /// `+0x468`, set by operation 0x37 Join Guild (`0x004F8560`). One of the four conditions
    /// for standing in that town's mayor election (`0x00528D65`).
    pub fn is_guild_member_in(&self, town_index: u8) -> bool {
        unsafe { self.get::<u32>(0x468) & (1u32 << town_index) != 0 }
    }

    pub fn get_first_office_index(&self) -> u16 {
        unsafe { self.get(0x0c) }
    }

    /// The head of this merchant's letter chain in the message pool; the next link is
    /// every letter's `+0x6`, and the chain ends on an index at or above the pool size.
    /// The tavern side room starts its search for mission offers here (`0x005A7223`).
    pub fn get_first_letter_index(&self) -> u16 {
        unsafe { self.get(0x0a) }
    }

    /// The head of this merchant's ship chain; the next link is every ship's `+0x4`
    /// (`ShipPtr::get_next_ship_index_of_merchant`), and the chain ends on an index at
    /// or above the ship count. Walking it costs this merchant's fleet instead of the
    /// world's ships - what the game's per-merchant ship census at `0x004F0AB1` does.
    pub fn get_first_ship_index(&self) -> u16 {
        unsafe { self.get(0x0e) }
    }

    /// This merchant's raw sailor pool in a town: one byte per town index, growing and
    /// shrinking with his sailor reputation (`+0x1F`).
    pub fn get_sailor_pool(&self, town_index: u8) -> u8 {
        unsafe { self.get(0xf0 + town_index as u32) }
    }

    /// The sailors this merchant can actually hire in a town, as the game computes it
    /// (`0x004F6CA0`, thiscall(merchant, town_index)): `min(sailor_pool, cap)` where the
    /// cap is the town's own `+0x2E4` minus one, and `0` when that cap drops below one.
    ///
    /// This is the number the tavern works from: its sailors page calls exactly this at
    /// `0x005D4CB1` with the window's town index (`window + 0x1BFC`), then caps what it
    /// offers at 50 - the immediate at `0x005D4CD6` that mod-tavern-show-all-sailors
    /// raises to 100.
    pub fn get_available_sailors(&self, town_index: u8) -> u8 {
        let available_sailors: extern "thiscall" fn(u32, u32) -> u8 = unsafe { mem::transmute(0x004F6CA0) };
        available_sailors(self.address, town_index as u32)
    }
}

impl P3Pointer for MerchantPtr {
    fn get_address(&self) -> u32 {
        self.address
    }
}
