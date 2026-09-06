use super::{p3_ptr::P3Pointer, storage::StoragePtr};

pub const OFFICE_SIZE: u32 = 0x44C;

/// How much of a ware a route ship may take from the office: `thiscall(office, ware) ->
/// units`, `ret 4`. The full stock (the storage's `+0x4` array), unless the office has an
/// administrator ([OfficePtr::has_administrator]), the ware's lock bit is set
/// ([OfficePtr::is_ware_locked]) and its minimum store (`+0x354`) is positive - then
/// `max(0, stock - minimum store)`. Called from one place, the route-stop load executor at
/// [TAKEABLE_CALL_SITE], which clamps the stop's order quantity to it and to the ship's
/// remaining capacity (`0x004D5879`..`0x004D5887`).
pub const TAKEABLE_ADDRESS: u32 = 0x0050_0EC0;
pub const TAKEABLE_CALL_SITE: u32 = 0x004D_5874;
/// The `call` at [TAKEABLE_CALL_SITE], for a hook to verify before it patches.
pub const TAKEABLE_CALL_ORIGINAL: [u8; 5] = [0xe8, 0x47, 0xb6, 0x02, 0x00];

#[derive(Debug, Clone, Copy)]
pub struct OfficePtr {
    pub address: u32,
}

impl OfficePtr {
    pub fn new(address: u32) -> Self {
        Self { address }
    }

    pub fn get_storage(&self) -> StoragePtr {
        StoragePtr::new(self.address)
    }

    pub fn get_merchant_index(&self) -> u16 {
        unsafe { self.get(0x2c4) }
    }

    /// The town the office stands in (`office+0x2C6`, read by the administrator paths at
    /// `0x004DD27B` and `0x0053DE9D`).
    pub fn get_town_index(&self) -> u8 {
        unsafe { self.get(0x2c6) }
    }

    /// The number of business buildings the owning merchant has in this town, one per
    /// building - verified across several saves and offices. Counted in a loop over the
    /// town's buildings (`0x004FFDD9`, `0x004FFE5A`), and added to the administrator's own
    /// wage by `0x00500F10` wherever the interface shows what the office pays him.
    pub fn get_business_building_count(&self) -> u16 {
        unsafe { self.get(0x2d2) }
    }

    /// What the trading office window shows as the administrator's wage: `0x00500F10` =
    /// the record's `field_C_daily_wage` plus [Self::get_business_building_count], or `0`
    /// when the office has no administrator.
    pub unsafe fn get_displayed_administrator_wage(&self) -> u32 {
        let func: extern "thiscall" fn(u32) -> u32 = std::mem::transmute(0x00500F10u32);
        func(self.address)
    }

    /// Head of this office's chain of [crate::data::merchant_building::MerchantBuildingPtr]
    /// records - the owning merchant's production buildings in this town. Walk it with
    /// [crate::data::merchant_building::resolve_merchant_building], following
    /// `get_next_index`, and stop once an index reaches the array count
    /// (`0x004DE63B` onward is the canonical walk). An empty chain is simply an
    /// out-of-range head.
    pub fn get_first_building_index(&self) -> u16 {
        unsafe { self.get(0x2cc) }
    }

    pub fn get_next_office_of_merchant_index(&self) -> u16 {
        unsafe { self.get(0x2c8) }
    }

    pub fn get_next_office_in_town_index(&self) -> u16 {
        unsafe { self.get(0x2ca) }
    }

    pub unsafe fn get_administrator_trade_prices(&self) -> [i32; 20] {
        self.get(0x2f4)
    }

    pub unsafe fn set_administrator_trade_prices(&self, prices: [i32; 20]) {
        self.set(0x2f4, &prices)
    }

    pub unsafe fn set_administrator_trade_actions(&self, actions: [i32; 20]) {
        self.set(0x2f4, &actions)
    }

    pub unsafe fn get_administrator_trade_stock(&self) -> [i32; 20] {
        self.get(0x354)
    }

    pub unsafe fn set_administrator_trade_stock(&self, stock: [i32; 20]) {
        self.set(0x354, &stock)
    }

    /// The office's administrator as an index into the auto-trader array, out of
    /// range (>= the auto-trader count) when none is employed - the bounds check the
    /// game itself does, e.g. before applying the administrator's buying discount at
    /// 0x004FF7C0.
    pub unsafe fn get_administrator_index(&self) -> u16 {
        self.get(0x2f2)
    }

    /// Whether an administrator is employed: the index is in range of the auto-trader
    /// array - the test [TAKEABLE_ADDRESS] makes at `0x00500EC9` against the count at
    /// `[0x006DD892]` ([crate::ships::SHIPS_ADDRESS] `+0xF2`).
    pub unsafe fn has_administrator(&self) -> bool {
        self.get_administrator_index() < crate::ships::ShipsPtr::new().get_auto_traders_size()
    }

    pub unsafe fn get_administrator_trade_lock_bitmap(&self) -> u32 {
        self.get(0x3b4)
    }

    /// Whether the ware's lock is set (bit `ware` of the bitmap at `+0x3B4`; operation
    /// `0x66` toggles it, the window's checkmark shows it).
    pub unsafe fn is_ware_locked(&self, ware: u32) -> bool {
        ware < 32 && self.get_administrator_trade_lock_bitmap() & (1 << ware) != 0
    }

    /// The game's own answer to "how much may a route ship take" - see [TAKEABLE_ADDRESS].
    pub unsafe fn takeable(&self, ware: u32) -> i32 {
        let func: extern "thiscall" fn(u32, u32) -> i32 = std::mem::transmute(TAKEABLE_ADDRESS);
        func(self.address, ware)
    }
}

impl P3Pointer for OfficePtr {
    fn get_address(&self) -> u32 {
        self.address
    }
}
