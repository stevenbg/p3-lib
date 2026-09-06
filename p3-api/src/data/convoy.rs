use super::p3_ptr::P3Pointer;

pub const CONVOY_SIZE: u32 = 0x3c;

pub struct ConvoyPtr {
    pub address: u32,
}

impl ConvoyPtr {
    pub fn new(address: u32) -> Self {
        Self { address }
    }

    /// The convoy's LEAD ship - the one that carries the convoy's trade route at
    /// `ship+0x132`, and whose name the game shows as the convoy's own.
    ///
    /// Measured in game: for a four-ship convoy, the
    /// lead was the only member with a route head; the other three read `0xFFFF`. That
    /// makes this the hop anything route-related needs, because the ship panel reports the
    /// clicked member and reports a member even when the convoy is selected as a whole.
    ///
    /// Do NOT enumerate members with [crate::ship::ShipPtr::get_next_ship_in_convoy]: that
    /// field is the ships-tick list link, and on the measured convoy it read `0xFFFF` on
    /// the lead. Scan the ships array for `+0x8 == convoy index` instead.
    pub fn get_lead_ship_index(&self) -> u16 {
        unsafe { self.get(0x10) }
    }

    /// The town the convoy lies in, or `0xFF` at sea. **Reads two bytes at an odd offset**,
    /// so the high half is whatever `+0x3a` holds - measured as 0 on a convoy at sea, which
    /// is the only reason this returns a clean `0x00FF` there. Treat it as a `u8` town index
    /// and never hand it to a lookup unchecked.
    pub fn get_current_town_index(&self) -> u16 {
        unsafe { self.get(0x39) }
    }

    pub fn get_status(&self) -> u16 {
        unsafe { self.get(0x12) }
    }
}

impl P3Pointer for ConvoyPtr {
    fn get_address(&self) -> u32 {
        self.address
    }
}
