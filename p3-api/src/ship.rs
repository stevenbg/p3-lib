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

    /// Also the link for the ships tick's two ship lists (`ships+0xE8` in port,
    /// `ships+0xEA` at sea): `0x00506720` walks them through this field, so the name
    /// is incomplete - a ship is spliced out of its list with
    /// `[cursor] = ship->0x6; ship->0x6 = 0xFFFF`. See
    /// `.claude/notes/done/port-freezing.md`.
    pub fn get_next_ship_in_convoy(&self) -> u16 {
        unsafe { self.get(0x06) }
    }

    pub fn get_convoy_id(&self) -> u16 {
        unsafe { self.get(0x08) }
    }

    /// Previous ship in the town's docking chain, `0xFFFF` when unlinked. A separate
    /// chain from the per-merchant (`+0x4`) and per-convoy (`+0x6`) ones: it is
    /// ordered by [ShipPtr::get_docking_sort_key], inserted by `0x0050D0C0` and
    /// unlinked by `0x0050D040(ships, index)` - a plain doubly-linked unlink that
    /// writes `0xFFFF` into both links and destroys nothing. See
    /// `.claude/notes/done/port-freezing.md`.
    pub fn get_previous_ship_in_port(&self) -> u16 {
        unsafe { self.get(0x0a) }
    }

    /// Next ship in the town's docking chain, `0xFFFF` when unlinked. See
    /// [ShipPtr::get_previous_ship_in_port].
    pub fn get_next_ship_in_port(&self) -> u16 {
        unsafe { self.get(0x0c) }
    }

    /// The signed key the docking chain is sorted by: `0x0050D0C0` walks the chain
    /// comparing this before linking a ship in. Meaning not yet identified.
    pub fn get_docking_sort_key(&self) -> i16 {
        unsafe { self.get(0x1e) }
    }

    /// The trade-route state byte. **Bit 0 is "automatic trade is running"**: the
    /// [ships tick](`0x00506720`) only executes a route stop for a ship with that bit
    /// set, [crate::operation::Operation::SetTradeRouteActive] writes the whole byte as
    /// `0x01` when activating (`0x0053E0F3`) and clears bits 0 and 1 when deactivating
    /// (`0x0053DF4D`), and a finished stop with action bit `0x02` clears the low bits
    /// again. Bit 0 is also the gate on the "destination port closed" check that turns a
    /// ship away from a frozen or blockaded port.
    pub fn get_trade_route_flags(&self) -> u8 {
        unsafe { self.get(0x136) }
    }

    /// Whether automatic trade is currently running for this ship - bit 0 of
    /// [ShipPtr::get_trade_route_flags].
    pub fn is_trade_route_active(&self) -> bool {
        self.get_trade_route_flags() & 1 != 0
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
