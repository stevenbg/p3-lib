//! A town's housing per citizen class: how many houses stand, how many people they hold
//! and how many they could - the "All dwellings in this town" block of the house info
//! panel.
//!
//! The panel's populate routine (`0x005B0250`..`0x005B0370`) fills three slots, poor,
//! wealthy, rich, from the town and its offices:
//!
//! - **capacity** = `word town[0x2FC - 2k]` (k = 0 poor, 1 wealthy, 2 rich; so rich at
//!   `+0x2F8`, wealthy `+0x2FA`, poor `+0x2FC`) - every house of the class in the town,
//!   whoever owns it;
//! - **occupants** = `word town[0x77C - 2k]` (residents of the town-owned houses) plus
//!   `word office[0x2E2 - 2k]` for every merchant with an office in the town
//!   (`0x005308A0` per merchant index, `0x005B02AE`..);
//! - **houses** = capacity / the class's per-house capacity, the word table at
//!   `0x00672A18`: rich 80, wealthy 140, poor 280 (`0x005B0298`).
//!
//! The panel skips a class whose capacity is 0 and prints `houses (occupants * 100 /
//! capacity)`, naming the poor's houses "Half-timbered", the wealthy's "Gabled" and the
//! rich's "Merchants'".

use crate::{data::p3_ptr::P3Pointer, game_world::GAME_WORLD_PTR};

use super::TownPtr;

/// The three housed classes, in the panel's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CitizenClass {
    Poor,
    Wealthy,
    Rich,
}

impl CitizenClass {
    pub const ALL: [CitizenClass; 3] = [CitizenClass::Poor, CitizenClass::Wealthy, CitizenClass::Rich];

    /// The game's name for the class's house type.
    pub fn house_name(self) -> &'static str {
        match self {
            CitizenClass::Poor => "Half-timbered houses",
            CitizenClass::Wealthy => "Gabled houses",
            CitizenClass::Rich => "Merchants' houses",
        }
    }

    /// The panel's slot index: the town and office arrays run rich to poor upwards, so
    /// each field is `base - 2 * k`.
    fn k(self) -> u32 {
        match self {
            CitizenClass::Poor => 0,
            CitizenClass::Wealthy => 1,
            CitizenClass::Rich => 2,
        }
    }

    /// People one house of this class holds (`0x00672A18` + 2 per class, rich first).
    pub fn per_house_capacity(self) -> u16 {
        unsafe { *((PER_HOUSE_CAPACITY_TABLE_ADDRESS + 4 - 2 * self.k()) as *const u16) }
    }
}

/// Word table of per-house capacities: rich, wealthy, poor (80, 140, 280).
pub const PER_HOUSE_CAPACITY_TABLE_ADDRESS: u32 = 0x0067_2A18;
/// Poor class's fields; the other classes sit 2 bytes below per step towards rich.
const TOWN_HOUSING_CAPACITY_POOR: u32 = 0x2FC;
const TOWN_HOUSING_RESIDENTS_POOR: u32 = 0x77C;
const OFFICE_HOUSING_RESIDENTS_POOR: u32 = 0x2E2;

/// One class's housing figures.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Dwellings {
    pub houses: i32,
    pub occupants: i32,
    pub capacity: i32,
}

impl Dwellings {
    /// `occupants * 100 / capacity` as the panel computes it; `None` when there is no
    /// capacity, which is when the panel leaves the line out.
    pub fn occupancy_percent(&self) -> Option<i32> {
        if self.capacity <= 0 {
            None
        } else {
            Some(self.occupants * 100 / self.capacity)
        }
    }
}

impl TownPtr {
    /// The housing figures of one class, summed the way the house info panel does it.
    pub unsafe fn get_dwellings(&self, class: CitizenClass) -> Dwellings {
        let k = class.k();
        let capacity = self.get::<u16>(TOWN_HOUSING_CAPACITY_POOR - 2 * k) as i32;
        let mut occupants = self.get::<u16>(TOWN_HOUSING_RESIDENTS_POOR - 2 * k) as i32;
        let mut office_index = self.get_first_office_index();
        while office_index < GAME_WORLD_PTR.get_offices_count() {
            let office = GAME_WORLD_PTR.get_office(office_index);
            occupants += office.get::<u16>(OFFICE_HOUSING_RESIDENTS_POOR - 2 * k) as i32;
            office_index = office.get_next_office_in_town_index();
        }
        let per_house = class.per_house_capacity() as i32;
        let houses = if per_house > 0 { capacity / per_house } else { 0 };
        Dwellings { houses, occupants, capacity }
    }
}
