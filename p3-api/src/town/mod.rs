use map::TownMapPtr;
use num_traits::FromPrimitive;
use shipyard::ShipyardPtr;

use crate::{
    data::{
        enums::{TownId, WareId},
        p3_ptr::P3Pointer,
        storage::StoragePtr,
    },
    facility::{FacilityPtr, FACILITY_SIZE},
    latin1_ptr_to_string,
};

pub mod construction;
pub mod dwellings;
pub mod beggars;
pub mod church;
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
/// The town has an active **plague** (identified 30 Aug 2026): scheduled task `0x1C`'s
/// handler (`0x004E9094`) sets it at `0x004E9453` while posting the "Outbreak of the
/// plague in %s" event (type `0x12`) and letters to every merchant, and clears it at
/// `0x004E9486` when the outbreak ends. Blocks beggar growth entirely (`0x0051C16D`),
/// and with it the feed-the-poor influx: the influx tests
/// `flags & 0x800008 == 0x800000`, so a plagued town gets nothing and does not even
/// consume the trigger bit.
pub const TOWN_FLAG_PLAGUE: u32 = 0x8;
/// The old name for [TOWN_FLAG_PLAGUE], from before the flag was identified - kept so
/// the effect-based name stays greppable.
pub const TOWN_FLAG_NO_BEGGAR_GROWTH: u32 = TOWN_FLAG_PLAGUE;
pub const TOWN_FLAG_WINTER: u32 = 0x2;
pub const TOWN_FLAG_SIEGE: u32 = 0x10;
pub const TOWN_FLAG_BLOCKADE: u32 = 0x200;
pub const TOWN_FLAG_PIRATE_ATTACK: u32 = 0x800;
/// The town is in **famine** (identified 31 Aug 2026). Set at `0x00527F2F` once the
/// food-shortage tally `town+0x2C3` passes `0x50`, cleared at `0x0052803A`; setting it
/// posts the "Famine in %s" event and letter. It **doubles the doubling** of a
/// celebration's ware consumption - `0x005004C0` tests bit 12 of the flags and scales
/// the per-guest amounts by 4 instead of 2.
pub const TOWN_FLAG_FAMINE: u32 = 0x1000;
/// The port is iced in. Set by the daily ice pass (`0x004E48CA`, which posts "The
/// port of %s is frozen.") and cleared by scheduled task `0x35` (`0x004E94A4`,
/// "The port of %s is open again."). See `.claude/notes/done/port-freezing.md`.
pub const TOWN_FLAG_FROZEN: u32 = 0x0400_0000;

/// Bits of the **built-structures mask** at `+0x76C` ([TownPtr::get_buildings]) - one
/// bit per unique town structure, set when the building is added to the town by
/// `0x00521900` (its 48-entry dispatch on the building id, index table `0x00522690`,
/// jump table `0x0052262C`). Each case refuses when its own bit is already set, which is
/// what makes these buildings one-per-town, and most also require other bits first - the
/// `(mask & X) == Y` form quoted on each constant, where `Y` is the prerequisite and
/// `X - Y` the building's own bit.
///
/// Named by decoding each case's own `or` and reading the id off the **name pointer table
/// at `0x006A57C8`** - 57 entries indexed by building id, which is the space this setter
/// bounds with its `cmp al,0x30`. That table is what names the low ids; the packed string
/// block it points into (`0x006A5688`) only covers `0x1E`..`0x2F`, which is why
/// [TOWN_BUILDING_REPAIR_DOCK] (id `0x01`) and [TOWN_BUILDING_WEAPONSMITH] (id `0x03`)
/// went unnamed before.
///
/// [TOWN_BUILDING_MINT] is corroborated a second way: the population-levels routine tests
/// exactly this bit at `0x0051C671` for the rich divisor the gitbook derived as
/// `has_mint`.
///
/// Bit `0x10` is a prerequisite of the Repair Dock, Tavern, Lender's House, Guild Hall and
/// Public Bath, and part of the Church's `0x7F` - but **nothing in the executable sets
/// it**: it arrives with the town, through the savegame/scenario read at `0x0051ECE0`.
/// Bits `0x100` (set at `0x004EA3E4`/`0x004EA440`) and `0x8000` (set at
/// `0x0041BDD8`/`0x0041BE3F`) are likewise unidentified. See
/// `.claude/notes/done/town-building-mask.md`.
/// No prerequisite; the only test is its own bit (`0x00521EB1`).
pub const TOWN_BUILDING_MARKET_HALL: u32 = 0x1;
/// `(mask & 0x3) == 0x1` - needs [TOWN_BUILDING_MARKET_HALL].
pub const TOWN_BUILDING_TOWN_HALL: u32 = 0x2;
/// Building id `0x03`. `(mask & 0x6) == 0x2` - needs [TOWN_BUILDING_TOWN_HALL], and is
/// itself what [TOWN_BUILDING_ARMOURY] needs.
pub const TOWN_BUILDING_WEAPONSMITH: u32 = 0x4;
/// `(mask & 0xC) == 0x4` - needs [TOWN_BUILDING_WEAPONSMITH] (`0x00521DBE`).
pub const TOWN_BUILDING_ARMOURY: u32 = 0x8;
/// `(mask & 0x30) == 0x10`.
pub const TOWN_BUILDING_TAVERN: u32 = 0x20;
/// Building id `0x01`, `(mask & 0x50) == 0x10` (`0x00521ABF`) - **the bit a town needs
/// before any ship can be repaired in it.** Both route-stop executors test it before
/// ordering the repair and *clear the stop's R flag* when it is missing: `0x00518993` for
/// a lone ship, `0x005032EA` for a convoy. A Shipyard does not replace it - the yard is
/// built on top of a finished dock ([TOWN_BUILDING_REPAIR_DOCK_COMPLETE]), so a town with
/// a Shipyard still carries this bit.
pub const TOWN_BUILDING_REPAIR_DOCK: u32 = 0x40;
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
/// `(mask & 0x2010) == 0x10`.
pub const TOWN_BUILDING_GUILD_HALL: u32 = 0x2000;
/// `(mask & 0x4010) == 0x10`.
pub const TOWN_BUILDING_PUBLIC_BATH: u32 = 0x4000;
/// Building id `0x2E`, `(mask & 0x10080) == 0x80` - needs a **completed** repair dock,
/// [TOWN_BUILDING_REPAIR_DOCK_COMPLETE], not merely a placed one.
pub const TOWN_BUILDING_SHIPYARD: u32 = 0x1_0000;
/// A Repair Dock or Shipyard site has **finished building**, as opposed to the two bits
/// above, which the setter writes when the site is placed. Set by the construction pass
/// `0x0051FF30` when a site of id `0x01` or `0x2E` completes (`0x00520127` for a site with
/// an owner, `0x0052072F` in its town-owned switch), and read at `0x00510210` to keep the
/// shipyard facility producing.
pub const TOWN_BUILDING_REPAIR_DOCK_COMPLETE: u32 = 0x80;
/// A **town-owned** Shipyard site has finished building (`0x0052036F`, `0x00520765`, the
/// town-owned halves of the same construction pass). This - not
/// [TOWN_BUILDING_SHIPYARD] - is what gates the yard's ship-build list at `0x0052B30A`,
/// and the weekly shipyard task `0x004E2144` reads it too.
pub const TOWN_BUILDING_SHIPYARD_COMPLETE: u32 = 0x2_0000;

pub const WARE_BASE_PRICES: *const f32 = 0x00673A18 as _;

/// A celebration's **per-guest base consumption**, one byte per ware, indexed by
/// [crate::data::enums::WareId] `0..8` (Grain..Wine): 3, 2, 2, 2, 0, 1, 0, 2. Salt and
/// Spices read `0` and are skipped by both readers, which is why a celebration can only
/// ever satisfy six wares - the hardcoded `/6` in its attendance math. Read by the
/// celebration task's attendance loop (`0x004E2491`) and by its consumption routine
/// (`0x005004C0`). See [TownPtr::get_celebration_consumption].
pub const CELEBRATION_BASE_CONSUMPTION_TABLE_ADDRESS: u32 = 0x006734E8;
/// How many wares the celebration tables cover: Grain..Wine.
pub const CELEBRATION_WARE_COUNT: usize = 8;

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

    /// What a celebration attended by `guests` guests eats out of the host's office, in
    /// raw units per ware - the game's own formula (`0x005004C0`, called by the
    /// celebration task at `0x004E2506` with the guest count): the per-guest base from
    /// [CELEBRATION_BASE_CONSUMPTION_TABLE_ADDRESS] times the guests, **doubled** - or
    /// quadrupled while the town is in [TOWN_FLAG_FAMINE] - then rounded up to whole
    /// in-game units. Zero for everything but Grain, Meat, Fish, Beer, Honey and Wine.
    ///
    /// This is twice what a ware needs for the celebration's **level** - see
    /// [TownPtr::get_celebration_level_requirement]. A ware that cannot cover its share is
    /// emptied to `0` (`0x005005A3`) and costs nothing beyond the stock, the level having
    /// been decided before any of it is removed.
    /// The office stock each ware must reach for a celebration to count it toward its
    /// **level**, in raw units: the attendance test at `0x004E2491`, `base * attendees`
    /// with neither the doubling of
    /// [TownPtr::get_celebration_consumption] nor any rounding - the raw product is
    /// compared straight against the stock. Six wares can qualify and every two of them
    /// are one level, so this is the whole cost of a top celebration; stocking more only
    /// feeds the doubled consumption, which buys nothing.
    ///
    /// `attendees` is the game's `citizens * attendance_ratio / 100` (the ratio is
    /// `39 + reputation`, capped at 99), so the full citizen count is the safe upper bound.
    pub fn get_celebration_level_requirement(&self, attendees: i32) -> [i32; 24] {
        let mut required = [0i32; 24];
        for (ware_index, amount) in required.iter_mut().enumerate().take(CELEBRATION_WARE_COUNT) {
            let base = unsafe { *((CELEBRATION_BASE_CONSUMPTION_TABLE_ADDRESS + ware_index as u32) as *const u8) } as i32;
            *amount = base.saturating_mul(attendees);
        }
        required
    }

    pub fn get_celebration_consumption(&self, guests: i32) -> [i32; 24] {
        let multiplier = if self.get_flags() & TOWN_FLAG_FAMINE != 0 { 4 } else { 2 };
        let mut consumption = [0i32; 24];
        for (ware_index, amount) in consumption.iter_mut().enumerate().take(CELEBRATION_WARE_COUNT) {
            let base = unsafe { *((CELEBRATION_BASE_CONSUMPTION_TABLE_ADDRESS + ware_index as u32) as *const u8) } as i32;
            let raw = base.saturating_mul(guests).saturating_mul(multiplier);
            if raw <= 0 {
                continue;
            }
            let scaling = WareId::from_u16(ware_index as u16).map_or(1, |ware_id| ware_id.get_scaling());
            *amount = (raw + scaling - 1) / scaling * scaling;
        }
        consumption
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

    /// Whether ships can be repaired here - the [TOWN_BUILDING_REPAIR_DOCK] test the
    /// game's own route-stop executors make before ordering a repair.
    pub fn can_repair_ships(&self) -> bool {
        self.has_building(TOWN_BUILDING_REPAIR_DOCK)
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

    /// The town's **total citizens**, all classes together (`town + 0x2D4`). The beggar
    /// routine recomputes it as the sum of the four class counts; the well's build limit
    /// (`0x005220C1`) and the feeding-the-poor gate both read it.
    pub fn get_citizens(&self) -> i32 {
        unsafe { self.get(0x2d4) }
    }

    /// Satisfaction per class, `town + 0x300` with stride 2 - **rich, wealthy, poor**, in
    /// that order. Signed: an unhappy class goes negative. The feeding-the-poor gate reads
    /// the poor entry (`town + 0x304`) with `movsx`.
    pub fn get_satisfactions(&self) -> [i16; 3] {
        unsafe { [self.get(0x300), self.get(0x302), self.get(0x304)] }
    }

    /// The town's **beggars** (`town + 0x2E4`) - the labour intake, outside the jobs
    /// identity. See [crate::town::beggars].
    pub fn get_beggars(&self) -> i32 {
        unsafe { self.get(0x2e4) }
    }

    /// Beggar satisfaction (`town + 0x306`) - the fourth entry of the satisfaction array,
    /// past the three classes [Self::get_satisfactions] covers. It drives the beggar
    /// equilibrium and is never lowered.
    pub fn get_beggar_satisfaction(&self) -> i16 {
        unsafe { self.get(0x306) }
    }

    /// The poor's satisfaction - [Self::get_satisfactions] index 2.
    pub fn get_poor_satisfaction(&self) -> i16 {
        self.get_satisfactions()[2]
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
