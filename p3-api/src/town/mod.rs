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
/// Slots in the game's per-town arrays, the name pointer table included. 40 dwords puts
/// that table at `0x006DDA00..0x006DDAA0`, comfortably short of the static town data at
/// [static_town_data::TOWN_DATA_ADDRESS] (`0x006DDB90`), and matches the 40 raw town ids
/// `GameWorldPtr::get_raw_town_ids` reads. Only the first `get_towns_count()` slots
/// describe a town that exists; the rest are a table, not a promise.
pub const TOWN_SLOTS: u8 = 40;

/// Bits of the town flag word at `+0x2C8` ([TownPtr::get_flags]). The four crisis
/// bits are the mask `0x04000A10` that `update_town_price_thresholds` tests at
/// `0x005280B7`.
pub const TOWN_FLAG_WINTER: u32 = 0x2;
pub const TOWN_FLAG_SIEGE: u32 = 0x10;
pub const TOWN_FLAG_BLOCKADE: u32 = 0x200;
pub const TOWN_FLAG_PIRATE_ATTACK: u32 = 0x800;
/// The port is iced in. Set by the daily ice pass (`0x004E48CA`, which posts "The
/// port of %s is frozen.") and cleared by scheduled task `0x35` (`0x004E94A4`,
/// "The port of %s is open again."). See `.claude/notes/done/port-freezing.md`.
pub const TOWN_FLAG_FROZEN: u32 = 0x0400_0000;

/// Bits of the **built-structures mask** at `+0x76C` ([TownPtr::get_buildings]) - one
/// bit per unique town structure, set when the building is added to the town by
/// `0x00521900` (its 48-entry dispatch on the building id, index table `0x00522690`,
/// jump table `0x0052262C`). Each case also refuses when its own bit is already set,
/// which is what makes these buildings one-per-town.
///
/// Named against the building-name block at `0x006A5688`..`0x006A5750`, which is in
/// building-id order and aligns on five independently known ids (Warehouse `0x1E`,
/// Hospital `0x29`, Mint `0x2A`, School `0x2B`, Chapel `0x2C`). [TOWN_BUILDING_MINT] is
/// corroborated a second way: the population-levels routine tests exactly this bit at
/// `0x0051C671` for the rich divisor the gitbook derived as `has_mint`.
pub const TOWN_BUILDING_MARKET_HALL: u32 = 0x1;
pub const TOWN_BUILDING_TOWN_HALL: u32 = 0x2;
pub const TOWN_BUILDING_ARMOURY: u32 = 0x8;
pub const TOWN_BUILDING_TAVERN: u32 = 0x20;
/// Prerequisite of both [TOWN_BUILDING_MINT] and [TOWN_BUILDING_SCHOOL], and itself
/// gated on all seven of `0x7F` (`(mask & 0x27F) == 0x7F` at `0x00521D39`).
pub const TOWN_BUILDING_CHURCH: u32 = 0x200;
/// Divides the rich-citizen target by 213 instead of 320 (`0x0051C671`), i.e. a +50%
/// rich-citizen target. Like [TOWN_BUILDING_SCHOOL] that is its *only* effect - three
/// readers in the executable, this one, the setter and the AI planner's "already has
/// one" test - so the Mint does nothing to money despite the name, which the player
/// independently confirmed from play. Requires [TOWN_BUILDING_CHURCH].
pub const TOWN_BUILDING_MINT: u32 = 0x400;
pub const TOWN_BUILDING_LENDERS_HOUSE: u32 = 0x800;
/// The only mechanical effect is on the beggar intake: the daily beggar pass
/// `0x0051C0E0` multiplies its *increase* step by 1.3 - `floor((13 * step + 9) / 10)`
/// at `0x0051C1B1` - and nothing else in the executable reads this bit except the
/// setter and the AI's "town already has one" test at `0x0051F893`. Requires
/// [TOWN_BUILDING_CHURCH]. See `.claude/notes/done/town-school.md`.
pub const TOWN_BUILDING_SCHOOL: u32 = 0x1000;
pub const TOWN_BUILDING_GUILD_HALL: u32 = 0x2000;
pub const TOWN_BUILDING_PUBLIC_BATH: u32 = 0x4000;
pub const TOWN_BUILDING_SHIPYARD: u32 = 0x1_0000;

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
    /// The town's flag word - see the `TOWN_FLAG_*` constants in this module.
    ///
    /// Bit `0x2` is winter, rewritten from the calendar by the town tick every day
    /// (`0x0051BA47`); the four crop producers scale their output by it. Bits
    /// `17..22` are masked and refilled by the same tick (`0x0051BD04`) and are
    /// unrelated. The word carries the town's crisis state too: siege, blockade,
    /// pirate attack and [TOWN_FLAG_FROZEN] are read together as the mask
    /// `0x04000A10` by `update_town_price_thresholds` (`0x005280B7`), which stretches
    /// t1 to 28 days and doubles the building-material factor when any is set.
    pub fn get_flags(&self) -> u32 {
        unsafe { self.get(0x2c8) }
    }

    /// Whether the town's port is iced in, i.e. [TOWN_FLAG_FROZEN] is set.
    pub fn is_port_frozen(&self) -> bool {
        self.get_flags() & TOWN_FLAG_FROZEN != 0
    }

    /// The mask of unique structures the town has, one bit each - see the
    /// `TOWN_BUILDING_*` constants. Written by `0x00521900` when a building is added;
    /// the town's ordinary houses, farms and workshops are not in here.
    pub fn get_buildings(&self) -> u32 {
        unsafe { self.get(0x76c) }
    }

    /// Whether the town has the given structure, e.g.
    /// `town.has_building(TOWN_BUILDING_SCHOOL)`.
    pub fn has_building(&self, building: u32) -> bool {
        self.get_buildings() & building != 0
    }

    /// Accumulated cold, the input to the ice model. The daily ice pass
    /// (`0x004E45C4`, scheduled task `0x0D`, run only when the day of the year is
    /// `<= 58` or `>= 333`) grows this in winter and melts it otherwise, then derives
    /// [TownPtr::get_ice_level] from it. A town can only freeze once this reaches
    /// `0x800`. See `.claude/notes/done/port-freezing.md`.
    pub fn get_cold_accumulator(&self) -> u32 {
        unsafe { self.get(0x9b8) }
    }

    /// Today's ice level, `(cold >> 9) + 2`, in the low 7 bits; bit `0x80` means the
    /// level is above 5, which is what makes the port eligible for the daily 1.5%
    /// freeze roll (`rand(0x6400) < 0x180`). Also the thaw delay: a port that freezes
    /// is scheduled to reopen `(level & 0x7F) + 1` days later. `+0x9BC` holds
    /// yesterday's level.
    pub fn get_ice_level(&self) -> u8 {
        unsafe { self.get(0x9bd) }
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
/// The name string for a town slot, or `None` when the slot names no town.
///
/// Both checks matter, and the second one was paid for: the table is filled at RUNTIME, so
/// a slot outside the current map's towns holds whatever was there - not reliably a null -
/// and walking a junk pointer as a latin1 string is an instant access violation. A probe
/// passing an at-sea convoy's town field (`0xFF`) straight in took the game down exactly
/// that way, faulting on `cmp byte [eax], 0` at an address that was never mapped. So
/// callers may pass any `u8`: a raw field out of a game record, an index from an unbounded
/// loop, a sentinel. This returns `None` rather than reading.
fn town_name_ptr(town_index: u8) -> Option<*const u8> {
    if town_index >= TOWN_SLOTS {
        return None;
    }
    unsafe {
        let town_names_ptr: *const *const u8 = TOWN_NAME_PTRS_ADDRESS as _;
        let name_ptr = *town_names_ptr.add(town_index as usize);
        crate::memory::is_readable(name_ptr as u32, 1).then_some(name_ptr)
    }
}

pub fn get_town_name_bytes(town_index: u8) -> Option<Vec<u8>> {
    let mut name_ptr = town_name_ptr(town_index)?;
    unsafe {
        let mut bytes = Vec::new();
        while *name_ptr != 0 {
            bytes.push(*name_ptr);
            name_ptr = name_ptr.add(1);
        }
        Some(bytes)
    }
}

pub fn get_town_name(town_index: u8) -> Option<String> {
    let name_ptr = town_name_ptr(town_index)?;
    unsafe { Some(latin1_ptr_to_string(name_ptr)) }
}
