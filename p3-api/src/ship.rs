use crate::{
    data::{enums::ShipType, p3_ptr::P3Pointer},
    latin1_to_string,
};

pub const SHIP_SIZE: u32 = 0x180;

#[derive(Debug, Clone, Copy)]
pub struct ShipPtr {
    pub address: u32,
}

impl ShipPtr {
    pub fn new(address: u32) -> Self {
        Self { address }
    }

    pub unsafe fn get_next_ship_index_of_merchant(&self) -> u16 {
        self.get(0x04)
    }

    /// The owning merchant (ships chain per owner via +0x4,
    /// `get_next_ship_index_of_merchant`).
    pub fn get_merchant_index(&self) -> u8 {
        unsafe { self.get(0x0) }
    }

    pub fn get_next_ship_in_convoy(&self) -> u16 {
        unsafe { self.get(0x06) }
    }

    pub fn get_convoy_id(&self) -> u16 {
        unsafe { self.get(0x08) }
    }

    pub fn get_type(&self) -> ShipType {
        unsafe { self.get(0x0e) }
    }

    pub fn get_capacity(&self) -> u32 {
        unsafe { self.get(0x10) }
    }

    pub fn get_max_health(&self) -> u32 {
        unsafe { self.get(0x14) }
    }

    pub fn get_current_health(&self) -> u32 {
        unsafe { self.get(0x18) }
    }

    pub fn get_x(&self) -> i32 {
        unsafe { self.get(0x1c) }
    }

    pub fn get_y(&self) -> i32 {
        unsafe { self.get(0x20) }
    }

    pub unsafe fn get_destination_town_index(&self) -> u8 {
        self.get(0x38)
    }

    pub fn get_last_town_index(&self) -> Option<u8> {
        let town_index: u8 = unsafe { self.get(0x39) };
        if town_index != 0xff {
            Some(town_index)
        } else {
            None
        }
    }

    pub fn get_wares(&self) -> [i32; 24] {
        unsafe { self.get(0x54) }
    }

    pub fn get_avg_prices(&self) -> [f32; 24] {
        unsafe { self.get(0xb4) }
    }

    pub fn get_payload_buy_sum(&self) -> i32 {
        unsafe { self.get(0x114) }
    }

    /// The ship's captain as an auto-trader index, out-of-range = none. The AI hire
    /// path (0x51a1b9) fills it from the town's tavern captain (resolver 0x5269a0)
    /// and unlinks the record from the town's chain.
    pub fn get_captain_index(&self) -> u16 {
        unsafe { self.get(0x42) }
    }

    pub fn get_status(&self) -> u16 {
        unsafe { self.get(0x134) }
    }

    /// Is the ship at the town named by `get_last_town_index`, rather than out at sea?
    ///
    /// The game classifies its own status field with exactly two tests, next to each
    /// other in the per-merchant ship census at `0x004F0B02`/`0x004F0B11`/`0x004F0B26`:
    /// `status == 0xF` is a merchant vessel at sea (the value mod-scrollmap-render-all-
    /// ships draws, along with `0x12` for an AI pirate), and `status <= 3` is the
    /// in-port family. `0` is lying in the port; `3` is set while entering it
    /// (`0x004E13FA`, which also clears the convoy fields `+0x6`/`+0x8` and ORs `0x60`
    /// into the flags at `+0x3C`), and `+0x39` already names the town then - verified
    /// in-game: a ship sailing to a town flips from `0xF` to `3` at the moment the town
    /// becomes enterable and its tavern reachable, before it has docked.
    pub fn is_in_port(&self) -> bool {
        self.get_status() <= 3
    }

    pub fn get_name(&self) -> String {
        let buf: [u8; 32] = unsafe { self.get(0x160) };
        latin1_to_string(&buf)
    }

    pub unsafe fn calc_free_capacity(&self) -> i32 {
        //TODO weapons, sailors
        self.get_capacity() as i32 - self.get_wares().iter().sum::<i32>() - 10000
    }
}

impl P3Pointer for ShipPtr {
    fn get_address(&self) -> u32 {
        self.address
    }
}
