use std::mem;

use crate::{data::p3_ptr::P3Pointer, latin1_ptr_to_string};

pub const MERCHANT_SIZE: u32 = 0x650;

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
