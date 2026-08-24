use super::p3_ptr::P3Pointer;

/// The game world object. Its array pointers and counts are what every `0x005303xx`
/// resolver indexes off, and the absolute addresses the mods use are just fields of it:
/// `+0x06` merchant building count (`0x006DE4A6`), `+0x08` office count (`0x006DE4A8`),
/// `+0x14` the day/quarter word (`0x006DE4B4`), `+0x68` towns array (`0x006DE508`),
/// `+0x70` merchant buildings array (`0x006DE510`), `+0x74` offices array
/// (`0x006DE514`).
pub const WORLD_ADDRESS: u32 = 0x006DE4A0;

/// One record per (merchant, town, facility type) - a merchant's own production
/// buildings, which are NOT entries in the town's 21-slot facility array.
pub const MERCHANT_BUILDING_SIZE: u32 = 0x14;

/// Pointer to the world-wide merchant building array (`world+0x70`).
pub const MERCHANT_BUILDINGS_ARRAY: *const u32 = 0x006DE510 as _;

/// How many slots of that array are live (`world+0x6`). 128 and 256 observed in two
/// saves, so the array grows - re-read it, do not cache it. Every caller of
/// [resolve_merchant_building] bounds-checks against this itself, because the resolver
/// does not.
pub const MERCHANT_BUILDING_COUNT: *const u16 = 0x006DE4A6 as _;

/// Resolve a merchant building index to its record, exactly as `0x005303B0` does:
/// `[world+0x70] + index * 0x14`, with **no bounds check**. Returns `None` for an index
/// at or past [MERCHANT_BUILDING_COUNT], which is also how a chain terminates.
pub fn resolve_merchant_building(index: u16) -> Option<MerchantBuildingPtr> {
    unsafe {
        if index >= *MERCHANT_BUILDING_COUNT {
            return None;
        }
        let base = *MERCHANT_BUILDINGS_ARRAY;
        if base == 0 {
            return None;
        }
        Some(MerchantBuildingPtr::new(base + index as u32 * MERCHANT_BUILDING_SIZE))
    }
}

/// A merchant's production buildings of one type in one town, aggregated into a single
/// record. The first 16 bytes mirror a town [crate::facility::FacilityPtr], but the
/// array stride is `0x14` and the record carries no merchant field: ownership is
/// implied by which office's chain holds it (head
/// [crate::data::office::OfficePtr::get_first_building_index], next
/// [Self::get_next_index]).
#[derive(Clone, Debug)]
pub struct MerchantBuildingPtr {
    pub address: u32,
}

impl MerchantBuildingPtr {
    pub fn new(address: u32) -> Self {
        Self { address }
    }

    /// Unlike a town facility this is **not** seeded from `BASE_EFFICIENCY`: it starts at
    /// a type-independent `1024` where the ware is effective in this town and `768` where
    /// it is not, then gains the same-type bonus for the merchant owning several of them
    /// (`+0%` at 1-2, `+3%` at 3-5, `+6%` at 6-8, `+10%` at 9 or more, truncated).
    pub fn get_efficiency(&self) -> u32 {
        unsafe { self.get(0x00) }
    }

    /// Workers currently employed across all the buildings of this record, never above
    /// [Self::get_capacity].
    pub fn get_employees(&self) -> u16 {
        unsafe { self.get(0x04) }
    }

    /// Only ever `0x04`..`0x14` - a merchant cannot own the four municipal types.
    pub fn get_type(&self) -> u8 {
        unsafe { self.get(0x06) }
    }

    pub fn get_town_index(&self) -> u8 {
        unsafe { self.get(0x07) }
    }

    /// The next record in the owning office's chain, terminating when it reaches
    /// [MERCHANT_BUILDING_COUNT] (`0x004DE660` and the other walk sites).
    pub fn get_next_index(&self) -> u16 {
        unsafe { self.get(0x08) }
    }

    /// Total worker capacity: the per-building capacity times the number of buildings.
    /// That capacity is **30 for most types and 15 for Brickworks and Pitchmaker**, so
    /// the building count - which decides the same-type bonus - is this divided by the
    /// right one of the two.
    pub fn get_capacity(&self) -> u16 {
        unsafe { self.get(0x0a) }
    }

    /// Summed alongside [Self::get_employees] by the walk at `0x004DE668`. Equal to
    /// [Self::get_capacity] in 183 of 185 measured records.
    pub fn get_field_c(&self) -> u16 {
        unsafe { self.get(0x0c) }
    }

    pub fn get_field_e(&self) -> u16 {
        unsafe { self.get(0x0e) }
    }
}

impl P3Pointer for MerchantBuildingPtr {
    fn get_address(&self) -> u32 {
        self.address
    }
}
