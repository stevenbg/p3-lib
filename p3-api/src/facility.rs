use crate::data::p3_ptr::P3Pointer;

pub const FACILITY_SIZE: u32 = 0x10;

/// Slots in a town's facility array at `town+0x840`. The town tick's facility loop
/// (`0x0051BB78`, `ebp = 0x15`) runs **21** iterations of stride `0x10`, so the array
/// spans `town+0x840`..`town+0x98F` - one slot per facility type id `0x00`..`0x14`,
/// including `Militia` (0), `Shipyard` (1) and the unused `0x02`.
pub const FACILITY_COUNT: u32 = 21;

#[derive(Clone, Debug)]
pub struct FacilityPtr {
    pub address: u32,
}

impl FacilityPtr {
    pub fn new(address: u32) -> Self {
        Self { address }
    }

    /// The facility's efficiency term, and for most types both production figures the town
    /// tick accumulates are built from it: the nominal one (`town+0x490`) as
    /// `efficiency * NOMINAL_WORKFORCE[type] * factor`, and the actual one
    /// (`storage+0xC4`) as `employees * efficiency * factor` (`0x0050EB30` and
    /// `0x0050EB5D`). The `factor` is a per-producer constant except in the four crop
    /// routines - grain, honey, wine and hemp - which pick theirs from three bits of the
    /// town flags at `town+0x2C8`.
    ///
    /// Three exceptions, all in the per-type producers rather than here:
    /// - **whale oil** is built from [crate::town::TownPtr::get_whaling_productivity]
    ///   instead, and reads only this facility's employees;
    /// - the **Weaponsmith** (type `0x03`) writes nothing into `town+0x490` for any weapon
    ///   ware, and its producer force-writes `1024` into this field whenever it reads `0`
    ///   (`0x0050F724`), so its efficiency is a hardcoded default rather than savegame
    ///   state;
    /// - a merchant's buildings do not use this field at all - see
    ///   [crate::data::merchant_building::MerchantBuildingPtr::get_efficiency].
    pub fn get_efficiency(&self) -> i32 {
        unsafe { self.get(0x00) }
    }

    /// Workers currently employed here. This is the term that makes `storage+0xC4`
    /// staffing-dependent while `town+0x490` is not; a facility with `0` employees is
    /// skipped outright by `0x005101D0` (except types `0` and `1`).
    pub fn get_employees(&self) -> u16 {
        unsafe { self.get(0x04) }
    }

    pub fn get_type(&self) -> u8 {
        unsafe { self.get(0x06) }
    }

    pub fn get_town_index(&self) -> u8 {
        unsafe { self.get(0x07) }
    }

    /// How productive this facility's ware is in this town, as a 1024-relative factor:
    /// `1024` = effective, `768` and `683` are the two "low" grades the town information
    /// window collapses into one, `0` = the town has no such facility.
    ///
    /// Town setup seeds [Self::get_efficiency] from this at `0x00545E48` as
    /// `BASE_EFFICIENCY[type] * productivity / 1024` (the u16 table of 21 entries at
    /// `0x00673C24`), so a town **created at runtime** - a player-founded settlement -
    /// follows that relation, as does any scenario that does not override the table. An
    /// established map town may carry authored values instead: in one 22-town save the 21
    /// map towns all differed from the table while the one founded town matched it exactly.
    /// Efficiency is per-town, per-facility savegame state: read it, do not compute it.
    ///
    /// Town setup writes this field from the scenario's two ware bitmaps at
    /// `0x00545912`: 17 iterations of one bit each over types `0x04..=0x14`
    /// (`1024` if the bit is in the effective map, `768` if in the ineffective one,
    /// `0` if in neither), with types `0x00..=0x03` hardcoded to `1024` just before
    /// at `0x005458EF`. Bit `0x20000` - the next bit after those 17 - is **whaling**,
    /// and it has no slot here: it lands in
    /// [crate::town::TownPtr::get_whaling_productivity] instead.
    pub fn get_productivity(&self) -> i16 {
        unsafe { self.get(0x08) }
    }

    /// A second worker counter, moved against [Self::get_employees] by the employment
    /// tail at `0x00510787` when the target count changes.
    pub fn get_field_a(&self) -> u16 {
        unsafe { self.get(0x0a) }
    }
}

/// `ware -> the facility type that produces it`, the u8 table of 24 at `0x00672C88`.
/// Read by the town information window's produced-wares lists (`0x005B7DAA` for the
/// effective list, `0x005B7EFD` for the low one), which skip any entry `<= 3`.
///
/// Every ware resolves to a real type except **whale oil**, whose entry is
/// [PRODUCER_TYPE_NONE]: whaling has no facility record at all. Its productivity
/// lives in a standalone town field instead - see
/// [crate::town::TownPtr::get_whaling_productivity]. Two wares share a producer with
/// another ware (meat and leather both `CattleFarm`, fish and whale oil both
/// `FishermansHouse`), and spices map to `Militia` (`0x00`) as "nobody produces
/// this" filler.
pub const PRODUCER_TYPE: *const [u8; 24] = 0x00672C88 as _;

/// The [PRODUCER_TYPE] sentinel for a ware with no facility record. Whale oil is the
/// only ware carrying it.
pub const PRODUCER_TYPE_NONE: u8 = 0xFF;

/// `facility type -> the ware it primarily produces`, the u8 table of 21 at
/// `0x00672C2C` - the inverse of [PRODUCER_TYPE], read at `0x0051014C` to find the
/// ware whose price thresholds decide whether the facility keeps running. Only
/// entries `0x03..=0x14` are meaningful; the first three read `0x06` (spices) as
/// filler. The secondary outputs (leather, whale oil) do not appear here.
pub const PRIMARY_WARE: *const [u8; 21] = 0x00672C2C as _;

/// The notional full workforce of each facility type, the u8 table of 21 at
/// `0x006735E8`: `250, 40, 25, 5, 72, 65, 68, 78, 60, 68, 60, 48, 60, 54, 70, 50,
/// 93, 58, 27, 26, 62`. It caps the worker target at `0x005101BF`.
///
/// For the ware producers the dispatcher at `0x005101D0` also passes the entry to the
/// producer as the multiplier that the *actual* production takes from
/// [FacilityPtr::get_employees] - which is why `town+0x490` is staffing-independent. The
/// values are constant-folded into those dispatch branches (`push 0x3c` for the apiary at
/// `0x00510415`, and so on); `FishermansHouse` is the one that computes it at runtime, 65
/// normally and 72 in a whaling town (`0x0051032C`, matching the same override at
/// `0x0051016A`).
///
/// The **Weaponsmith** (type `0x03`) uses its entry only as the worker target: its branch
/// passes its producer the armament to work on rather than a workforce, and writes nothing
/// into `town+0x490`.
pub const NOMINAL_WORKFORCE: *const [u8; 21] = 0x006735E8 as _;

/// The per-type branches the town tick dispatches to, the jump table of 21 at
/// `0x00510880` inside `0x005101D0`. Handy when chasing where a ware's output actually
/// comes from: `FishermansHouse` is `0x0051032C`, whose producer `0x0050E690` is the one
/// that reads [crate::town::TownPtr::get_whaling_productivity].
///
/// Most branches just call their type's producer, but not all: the `Weaponsmith` branch
/// `0x005102C2` first calls `0x00510AA0` to pick which armament the town should work on,
/// and only then hands that to its producer `0x0050F6E0`.
pub const PRODUCER_DISPATCH_TABLE: u32 = 0x00510880;

impl P3Pointer for FacilityPtr {
    fn get_address(&self) -> u32 {
        self.address
    }
}

/// The four crop wares whose producers read the town flags at `town + 0x2C8`: grain,
/// honey, wine and hemp. The other seventeen producers fold a constant instead, checked
/// over the full extent of all twenty-one routines.
pub const CROP_WARES: [u8; 4] = [0, 5, 7, 17];

/// Bit `0x2` of `town + 0x2C8` - **winter**. The town tick sets it when the month is below
/// 2 or above 10 (`0x0051BA1C`/`0x0051BA29`), i.e. December, January and February, and
/// clears it otherwise, every town every day.
pub const TOWN_FLAG_WINTER: u32 = 0x2;
/// Two further bits the crop ladders read. Never observed set in any measured save, so what
/// they mean is unknown - only that they would deepen or invert the winter effect.
pub const TOWN_FLAG_CROP_A: u32 = 0x2000;
pub const TOWN_FLAG_CROP_B: u32 = 0x4000;

/// The integer factor a crop producer folds into its divisor, given the town's
/// `+0x2C8` flags. Output is proportional to this, so the ratio between two flag states is
/// the ratio between the outputs.
///
/// Each of the four has its own ladder, read from its disassembly - they are not uniform:
/// grain's [TOWN_FLAG_CROP_A] divides by 3 where the apiary's divides by 2, the vineyard
/// doubles where the apiary triples, and hemp selects a value outright instead of scaling a
/// base. `None` for any ware that is not one of [CROP_WARES].
pub fn crop_factor(ware: u8, flags: u32) -> Option<u32> {
    let winter = flags & TOWN_FLAG_WINTER != 0;
    let a = flags & TOWN_FLAG_CROP_A != 0;
    let b = flags & TOWN_FLAG_CROP_B != 0;
    Some(match ware {
        // FarmGrain 0x0050EAD0
        0 => {
            let base = if winter { 4 } else { 6 };
            if a {
                base / 3
            } else if b {
                base * 2
            } else {
                base
            }
        }
        // Apiary 0x0050EA00 and Vineyard 0x0050F100 - same base, different `b` arm.
        5 | 7 => {
            let base = if winter { 2 } else { 4 };
            if a {
                base / 2
            } else if b {
                if ware == 5 {
                    base * 3
                } else {
                    base * 2
                }
            } else {
                base
            }
        }
        // FarmHemp 0x0050EBF0 - a selection, so `winter` wins outright.
        17 => {
            if winter {
                3
            } else if a {
                4
            } else if b {
                9
            } else {
                6
            }
        }
        _ => return None,
    })
}

/// What winter costs a crop in this town, as a percentage of its non-winter output:
/// `crop_factor(winter) * 100 / crop_factor(summer)`, computed from the town's other flag
/// bits as they actually stand. With those bits clear - which is every save measured so far
/// - it is 66% for grain and 50% for honey, wine and hemp.
pub fn crop_winter_percent(ware: u8, flags: u32) -> Option<u32> {
    let summer = crop_factor(ware, flags & !TOWN_FLAG_WINTER)?;
    let winter = crop_factor(ware, flags | TOWN_FLAG_WINTER)?;
    if summer == 0 {
        return None;
    }
    Some(winter * 100 / summer)
}
