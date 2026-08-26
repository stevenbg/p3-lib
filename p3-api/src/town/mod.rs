use map::TownMapPtr;
use shipyard::ShipyardPtr;

use crate::{
    data::{enums::TownId, p3_ptr::P3Pointer, storage::StoragePtr},
    facility::{FacilityPtr, FACILITY_SIZE},
    latin1_ptr_to_string,
};

pub mod map;
pub mod shipyard;
pub mod static_town_data;

pub const TOWN_SIZE: u32 = 0x9F8;
pub const TOWN_NAME_PTRS_ADDRESS: u32 = 0x006DDA00;
pub const WARE_BASE_PRICES: *const f32 = 0x00673A18 as _;

#[derive(Debug)]
pub struct TownPtr {
    pub address: u32,
}

impl TownPtr {
    pub fn new(address: u32) -> Self {
        Self { address }
    }

    pub fn get_storage(&self) -> StoragePtr {
        StoragePtr::new(self.address)
    }

    pub unsafe fn get_town_id(&self) -> TownId {
        self.get(0x2c1)
    }

    pub unsafe fn get_town_id_u8(&self) -> u8 {
        self.get(0x2c1)
    }

    pub fn get_daily_consumptions_citizens(&self) -> [i32; 24] {
        unsafe { self.get(0x310) }
    }

    /// Daily consumption of the town's businesses, raw units per day. Identified in
    /// update_town_price_thresholds (0x528070), whose core loop computes the documented
    /// t0 = 7 days x (business + citizen consumption + 1) as
    /// [town+0x64+i*4] + [town+0x310+i*4] + 1.
    pub fn get_daily_consumptions_businesses(&self) -> [i32; 24] {
        unsafe { self.get(0x64) }
    }

    /// The town's NOMINAL daily production in raw units: capacity at full
    /// utilization. Facilities count by existence - staffing is ignored entirely,
    /// verified down to 0% utilization (the market hall window shows the actual
    /// staffing-scaled output instead). Nonzero exactly for the wares the town
    /// produces; the price threshold t2 is t1 + 10 days of this.
    /// The town's flag word. Bit `0x2` is winter, rewritten from the calendar by the town
    /// tick every day (`0x0051BA47`); the four crop producers scale their output by it, and
    /// nothing else in the game reads it. Bits `17..22` are masked and refilled by the same
    /// tick (`0x0051BD04`) and are unrelated.
    pub fn get_flags(&self) -> u32 {
        unsafe { self.get(0x2c8) }
    }

    pub fn get_production_values(&self) -> [i32; 24] {
        unsafe { self.get(0x490) }
    }

    pub fn get_unknown_stock(&self) -> [i32; 24] {
        unsafe { self.get(0x670) }
    }

    /// The head of this town's auto-trader chain (records linked via their
    /// `get_next_index`, ended by an out-of-range index; `0xFFFF` = empty, sentinel
    /// write 0x525f08). The chain mixes two record kinds: the town's tavern captain
    /// (when one is here) and the town's pirate captain (the tavern pirate a ship
    /// can be handed to). A hireable tavern captain is a chain record with
    /// `is_captain()` and merchant `0xFF` - the game's captain resolver
    /// 0x5269a0(town, merchant) walks the chain applying exactly that, preferring a
    /// captain the asking merchant employs; the sibling resolver 0x5261d0 does the
    /// same for the other record kind.
    pub fn get_auto_trader_chain_head(&self) -> u16 {
        unsafe { self.get(0x82e) }
    }

    pub fn get_councillor_bribes(&self) -> [u8; 4] {
        unsafe { self.get(0x6dc) }
    }

    pub unsafe fn get_first_office_index(&self) -> u16 {
        self.get(0x784)
    }

    pub fn get_town_map(&self) -> TownMapPtr {
        TownMapPtr { address: self.address + 0x7a4 }
    }

    pub fn get_shipyard(&self) -> ShipyardPtr {
        ShipyardPtr::new(self.address + 0x810)
    }

    /// The town's **whaling** productivity, `1024` / `768` / `0` on the same scale as
    /// [FacilityPtr::get_productivity] - but stored here, outside the 21-slot facility
    /// array, because whaling has no facility record. Written by town setup at
    /// `0x00545961` from bit `0x20000` of the scenario's two ware bitmaps, the bit
    /// straight after the 17 that cover facility types `0x04..=0x14`; the same branch
    /// also forces `FishermansHouse` productivity (`town+0x898`) down to `768`, which
    /// is why a whaling town's fish output is the lower grade.
    ///
    /// This field stands in for the missing facility's *efficiency* as well: the
    /// producer at `0x0050E690` computes whale oil as
    /// `employees * whaling_productivity * 27 / 1024` off the fisherman's hut's
    /// workforce (`0x0050E753`), while fish next to it uses
    /// [FacilityPtr::get_efficiency]. Whale oil therefore borrows the hut's employees but
    /// not its efficiency, and the town information window lists it on the strength of
    /// this field alone, because both of its lists special-case
    /// [crate::facility::PRODUCER_TYPE_NONE] and read this field directly
    /// (`0x005B7E41`, `0x005B7FA1`, threshold `1000`).
    pub fn get_whaling_productivity(&self) -> i32 {
        unsafe { self.get(0x2cc) }
    }

    pub fn get_facility(&self, index: u32) -> FacilityPtr {
        FacilityPtr::new(self.address + 0x840 + FACILITY_SIZE * index)
    }

    pub unsafe fn get_price_thresholds(&self) -> [[i32; 4]; 24] {
        self.get(0x4f0)
    }
}

impl P3Pointer for TownPtr {
    fn get_address(&self) -> u32 {
        self.address
    }
}

/// The town-name bank holds this many slots; maps with fewer towns repeat the first
/// town's pointer in the unfilled slots.
pub const TOWN_NAME_SLOTS: u8 = 40;

/// A town's name as its raw latin1 bytes, straight from the name bank - for
/// byte-exact work against other game strings (matching, splicing), where decoding
/// to UTF-8 would corrupt the comparison.
pub fn get_town_name_bytes(town_index: u8) -> Option<Vec<u8>> {
    unsafe {
        let town_names_ptr: *const *const u8 = TOWN_NAME_PTRS_ADDRESS as _;
        let mut name_ptr = *town_names_ptr.add(town_index as _);
        if name_ptr.is_null() {
            return None;
        }
        let mut bytes = Vec::new();
        while *name_ptr != 0 {
            bytes.push(*name_ptr);
            name_ptr = name_ptr.add(1);
        }
        Some(bytes)
    }
}

pub fn get_town_name(town_index: u8) -> Option<String> {
    unsafe {
        let town_names_ptr: *const *const u8 = TOWN_NAME_PTRS_ADDRESS as _;
        let town_name_ptr = *town_names_ptr.add(town_index as _);
        if town_name_ptr.is_null() {
            None
        } else {
            Some(latin1_ptr_to_string(town_name_ptr))
        }
    }
}
