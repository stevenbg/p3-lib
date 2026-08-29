use crate::data::p3_ptr::P3Pointer;

#[derive(Debug)]
#[repr(C)]
pub struct ShipLevels {
    pub snaikka_level: i8,
    pub crayer_level: i8,
    pub cog_level: i8,
    pub hulk_level: i8,
}

#[derive(Debug)]
pub struct ShipyardPtr {
    pub address: u32,
}

impl ShipyardPtr {
    pub fn new(address: u32) -> Self {
        Self { address }
    }

    pub fn get_experience(&self) -> u32 {
        unsafe { self.get(0x00) }
    }

    /// Experience earned but not yet banked. Every work tick credits the work each queued
    /// ship received here (`0x00508B43`, `0x00508C2D`, `0x00508C6F`), and the daily pass
    /// `0x004E2150` moves the whole amount into [ShipyardPtr::get_experience] and zeroes
    /// it. Because it is credited PER SHIP, a yard with several ships in it earns
    /// experience several times as fast.
    pub fn get_pending_experience(&self) -> u32 {
        unsafe { self.get(0x04) }
    }

    /// The busy-yard price markup, an exponential moving average maintained by the
    /// weekly task `0x06` (`0x004E2144`):
    /// `new = 0.961538 * old + (construction_capacity / 2000 + pending_experience / 19600) / 26`.
    /// Both the repair cost (`0x0052AA28`) and the ship build cost (`0x0052B95E`) step
    /// off it in bands.
    ///
    /// The capacity term walks [ShipyardPtr::get_construction_queue_head] only, but the
    /// second term is [ShipyardPtr::get_pending_experience] - which repairs credit PER
    /// SHIP - so a yard kept busy with repairs does get dearer too, just through the
    /// experience term rather than the capacity one.
    pub fn get_utilization_markup(&self) -> f32 {
        unsafe { self.get(0x08) }
    }

    /// Head of the chain of ships under CONSTRUCTION here (status `0x0E`), `0xFFFF` when
    /// empty. Ships are appended at the tail by `0x00507EA4` and linked through
    /// `ship+0x6` - the same multi-purpose link the tick lists and the convoy chain use.
    ///
    /// A chain, not one ship: the old name `get_building_ship_index` read as a single
    /// slot and was wrong. `0x005083B0` is its work pass, and `0x0052AFC0` is the
    /// shipyard window's ETA, which sums the remaining work of every ship AHEAD of the
    /// queried one - so construction really does queue.
    pub fn get_construction_queue_head(&self) -> u16 {
        unsafe { self.get(0x0c) }
    }

    /// Head of the chain of ships under REPAIR here (status `6`), `0xFFFF` when empty.
    /// Appended at the tail by `0x0052AC80`, linked through `ship+0x6`.
    ///
    /// Its work pass `0x00508AA0(town, work)` walks the WHOLE chain and adds the same
    /// `work` to every ship's health at `ship+0x18` - the amount is re-read from the
    /// stack each iteration and never divided or decremented. So repairs at one yard do
    /// not compete: ten ships mend exactly as fast as one. See
    /// `.claude/notes/done/ship-repair.md`.
    pub fn get_repair_queue_head(&self) -> u16 {
        unsafe { self.get(0x0e) }
    }

    /// Head of a third ship chain, purpose not identified; initialised to `0xFFFF` beside
    /// the other two by the town reset at `0x0051B1A7` and saved with them, but no work
    /// pass has been found for it.
    pub fn get_third_queue_head(&self) -> u16 {
        unsafe { self.get(0x10) }
    }

    pub fn get_current_quality_levels(&self) -> ShipLevels {
        unsafe { self.get(0x14) }
    }

    pub fn get_always_zero(&self) -> [u8; 4] {
        unsafe { self.get(0x18) }
    }
}

impl P3Pointer for ShipyardPtr {
    fn get_address(&self) -> u32 {
        self.address
    }
}
