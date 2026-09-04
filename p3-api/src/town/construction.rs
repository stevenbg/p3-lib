//! A town's construction sites: what is being built, by whom, and how far along.
//!
//! The site array hangs off the town (`+0x75C` base, 8-byte records, `+0x776` bound).
//! Sites still being built form a chain from the cursor `town+0x762` through each record's
//! `+0x4`, ending at an index at or past the bound; the daily construction pass
//! `0x0051FF30` walks exactly this chain. Finished sites leave it for the owner's or the
//! town's chains of completed structures.
//!
//! A site's `+0x6` is the **work left**, in the units the town's building workforce pays:
//! each day the pass spends the workforce's employee count as a budget over the chain -
//! a site whose remainder fits the budget is completed, any other is paid down by up to 5
//! (`0x00520479`..`0x00520493`). The building's total is the byte table `0x00672BB4`
//! indexed by building id, which the pass also hands the map renderer for the site's
//! progress picture (`0x005204B7`).

use crate::data::{enums::FacilityId, p3_ptr::P3Pointer};

use super::TownPtr;

/// The most a day's budget pays into a site it cannot finish (`0x0052048E`).
pub const CONSTRUCTION_MAX_PAYMENT_PER_DAY: u8 = 5;
/// `thiscall(town, site_index) -> i32`, `ret 4`: the game's own "remaining building time",
/// the number the building info panels print (`0x005A881B`, `0x005B0234`). It walks the
/// pending chain to the site, letting each site ahead take up to 5 of today's budget; if the
/// budget is gone before the site it returns a **negative** number: `-1` for the first
/// unfunded site (the panel says "Building will start soon"), `-n` for the n-th (the panel
/// says "This building is in position n. in the order for completion", `0x005A0FBA`
/// negates it; at or below `-1000` it prints nothing). Otherwise
/// `ceil(work_left / min(budget, 5))`, minus one when this town's daily pass has already
/// run today (`0x0052DFF0`: the town's slot is tick `town_index * 8 + 7` of the day). `0`
/// for a site with no work left, `0x10000000` for an index out of range.
pub const CONSTRUCTION_DAYS_REMAINING: u32 = 0x0051_D6F0;

pub const CONSTRUCTION_SITE_SIZE: u32 = 8;
/// Total work units per building id, one byte each, indexed like the name table.
pub const CONSTRUCTION_WORK_TABLE_ADDRESS: u32 = 0x0067_2BB4;
/// The building name pointer table: 57 entries indexed by building id, each a pointer to
/// the latin1 C string the game shows (`0x006A57C8`).
pub const BUILDING_NAME_TABLE_ADDRESS: u32 = 0x006A_57C8;
pub const BUILDING_ID_COUNT: u8 = 57;

const TOWN_SITES_BASE: u32 = 0x75C;
const TOWN_SITES_PENDING_HEAD: u32 = 0x762;
const TOWN_SITES_BOUND: u32 = 0x776;

/// One construction site record.
#[derive(Clone, Copy, Debug)]
pub struct ConstructionSitePtr {
    pub address: u32,
}

impl P3Pointer for ConstructionSitePtr {
    fn get_address(&self) -> u32 {
        self.address
    }
}

impl ConstructionSitePtr {
    pub fn get_map_x(&self) -> u8 {
        unsafe { self.get(0x0) }
    }
    pub fn get_map_y(&self) -> u8 {
        unsafe { self.get(0x1) }
    }
    /// The owning merchant; a value at or above the merchant count means the town owns it.
    pub fn get_owner_merchant_index(&self) -> u8 {
        unsafe { self.get(0x2) }
    }
    /// `1..0x38`, indexing [BUILDING_NAME_TABLE_ADDRESS] and [CONSTRUCTION_WORK_TABLE_ADDRESS].
    pub fn get_building_id(&self) -> u8 {
        unsafe { self.get(0x3) }
    }
    pub fn get_next_index(&self) -> u16 {
        unsafe { self.get(0x4) }
    }
    /// Work units still to pay before the building completes.
    pub fn get_work_left(&self) -> u8 {
        unsafe { self.get(0x6) }
    }
    /// The building's total work units, from the game's table.
    pub fn get_work_total(&self) -> u8 {
        building_work_units(self.get_building_id())
    }
}

/// Total work units to build the given building id, 0 for an id out of the table.
pub fn building_work_units(building_id: u8) -> u8 {
    if building_id >= BUILDING_ID_COUNT {
        return 0;
    }
    unsafe { *((CONSTRUCTION_WORK_TABLE_ADDRESS + building_id as u32) as *const u8) }
}

/// The game's own name for a building id, as latin1 bytes; `None` for an id out of the table.
pub fn get_building_name(building_id: u8) -> Option<Vec<u8>> {
    if building_id >= BUILDING_ID_COUNT {
        return None;
    }
    unsafe {
        let pointer = *((BUILDING_NAME_TABLE_ADDRESS + 4 * building_id as u32) as *const u32);
        if pointer == 0 {
            return None;
        }
        let mut bytes = Vec::new();
        let mut p = pointer as *const u8;
        while *p != 0 {
            bytes.push(*p);
            p = p.add(1);
        }
        Some(bytes)
    }
}

impl TownPtr {
    /// The sites still being built with their array indices, in the chain's order (the
    /// order the daily pass pays them in). Bounded by the array size, so a corrupt link
    /// cannot spin.
    pub unsafe fn get_pending_construction_sites(&self) -> Vec<(u16, ConstructionSitePtr)> {
        let base: u32 = self.get(TOWN_SITES_BASE);
        let bound: u16 = self.get(TOWN_SITES_BOUND);
        let mut sites = Vec::new();
        if base == 0 {
            return sites;
        }
        let mut index: u16 = self.get(TOWN_SITES_PENDING_HEAD);
        for _ in 0..bound {
            if index >= bound {
                break;
            }
            let site = ConstructionSitePtr { address: base + index as u32 * CONSTRUCTION_SITE_SIZE };
            sites.push((index, site));
            index = site.get_next_index();
        }
        sites
    }

    /// The game's remaining building time for a site, as the info panel shows it - see
    /// [CONSTRUCTION_DAYS_REMAINING] for the raw value's meaning.
    pub unsafe fn get_construction_days_remaining(&self, site_index: u16) -> ConstructionEta {
        let f: extern "thiscall" fn(u32, u32) -> i32 = std::mem::transmute(CONSTRUCTION_DAYS_REMAINING);
        match f(self.address, site_index as u32) {
            0x1000_0000 => ConstructionEta::Invalid,
            0 => ConstructionEta::Finishing,
            -1 => ConstructionEta::StartsSoon,
            raw if raw <= -1000 => ConstructionEta::Invalid,
            raw if raw < 0 => ConstructionEta::Queued { position: (-raw) as u32 },
            days => ConstructionEta::Days(days as u32),
        }
    }

    /// How many builders today's pass puts on each pending site, in chain order - the
    /// accounting [CONSTRUCTION_DAYS_REMAINING] uses: each site takes up to
    /// [CONSTRUCTION_MAX_PAYMENT_PER_DAY] of what is left of the budget, so the sites at the
    /// end of a long chain get none.
    pub unsafe fn get_construction_builders_assigned(&self) -> Vec<u16> {
        let mut budget = self.get_construction_budget();
        self.get_pending_construction_sites()
            .iter()
            .map(|_| {
                let assigned = budget.min(CONSTRUCTION_MAX_PAYMENT_PER_DAY as u16);
                budget -= assigned;
                assigned
            })
            .collect()
    }

    /// The daily construction budget: the employee count of the town's building workforce
    /// (facility type [FacilityId::Construction]), which the pass spends over the chain.
    pub fn get_construction_budget(&self) -> u16 {
        self.get_facility(FacilityId::Construction as u32).get_employees()
    }

}

/// The game's answer to "when will this site finish", see [CONSTRUCTION_DAYS_REMAINING].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConstructionEta {
    /// So many days, as the info panel prints ("Remaining building time: N days").
    Days(u32),
    /// No work left: the next pass completes it.
    Finishing,
    /// The first site today's budget does not reach ("Building will start soon").
    StartsSoon,
    /// Further down the unfunded part of the chain: the panel's "position n. in the order
    /// for completion".
    Queued { position: u32 },
    /// The site index is out of range, or the game's value is one the panel leaves blank.
    Invalid,
}
