use super::{p3_ptr::P3Pointer, storage::StoragePtr};

pub const OFFICE_SIZE: u32 = 0x44C;

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

    pub unsafe fn get_administrator_trade_lock_bitmap(&self) -> u32 {
        self.get(0x3b4)
    }
}

impl P3Pointer for OfficePtr {
    fn get_address(&self) -> u32 {
        self.address
    }
}
