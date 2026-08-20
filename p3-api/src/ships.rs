use crate::{
    auto_trader::{AutoTraderPtr, AUTO_TRADER_SIZE},
    data::{
        convoy::{ConvoyPtr, CONVOY_SIZE},
        p3_ptr::P3Pointer,
    },
};

use super::ship::{ShipPtr, SHIP_SIZE};

pub const SHIPS_ADDRESS: u32 = 0x006dd7a0;

#[derive(Clone, Debug)]
pub struct ShipsPtr {
    pub address: u32,
}

impl Default for ShipsPtr {
    fn default() -> Self {
        Self::new()
    }
}

impl ShipsPtr {
    pub const fn new() -> Self {
        Self { address: SHIPS_ADDRESS }
    }

    pub fn get_ship(&self, ship_id: u16) -> Option<ShipPtr> {
        if ship_id < self.get_ships_size() {
            let base_address: u32 = unsafe { self.get(0x04) };
            Some(ShipPtr::new(base_address + ship_id as u32 * SHIP_SIZE))
        } else {
            None
        }
    }

    pub fn get_ship_by_name(&self, name: &str) -> Option<(ShipPtr, u16)> {
        for i in 0..self.get_ships_size() {
            let ship = self.get_ship(i).unwrap();
            if name == ship.get_name() {
                return Some((ship, i));
            }
        }
        None
    }

    pub fn get_convoy(&self, convoy_id: u16) -> Option<ConvoyPtr> {
        if convoy_id < self.get_convoys_size() {
            let base_address: u32 = unsafe { self.get(0x08) };
            Some(ConvoyPtr::new(base_address + convoy_id as u32 * CONVOY_SIZE))
        } else {
            None
        }
    }

    /// The auto-trader array (captains and administrators): pointer at +0x0, one
    /// record per hired or hireable auto trader.
    pub fn get_auto_trader(&self, index: u16) -> Option<AutoTraderPtr> {
        if index < self.get_auto_traders_size() {
            let base_address: u32 = unsafe { self.get(0x00) };
            Some(AutoTraderPtr::new(base_address + index as u32 * AUTO_TRADER_SIZE))
        } else {
            None
        }
    }

    pub fn get_auto_traders_size(&self) -> u16 {
        unsafe { self.get(0xf2) }
    }

    pub fn get_ships_size(&self) -> u16 {
        unsafe { self.get(0xf4) }
    }

    pub fn get_convoys_size(&self) -> u16 {
        unsafe { self.get(0xf6) }
    }
}

impl P3Pointer for ShipsPtr {
    fn get_address(&self) -> u32 {
        self.address
    }
}
