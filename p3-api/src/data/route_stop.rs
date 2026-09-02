//! Trade-route stops as the running game holds them.
//!
//! A route is a **circular chain** of 220-byte records in a global pool, the same layout
//! as a `.rou` file's stops except that the file's two unused leading bytes carry the
//! next-stop pool index at runtime. A ship names its current stop in `ship+0x132`, and
//! the stop carrying action bit [ACTION_FIRST_STOP] is the route's logical first.

use crate::data::p3_ptr::P3Pointer;

/// Pool base pointer, and the record count beside it.
pub const ROUTE_STOP_POOL_PTR_ADDRESS: *const u32 = 0x006D_D72C as _;
pub const ROUTE_STOP_POOL_SIZE_ADDRESS: *const u16 = 0x006D_D72A as _;
pub const ROUTE_STOP_SIZE: u32 = 220;

/// Set on the route's logical first stop. The stop executor consults it to file the
/// first-stop report (`0x004D5200`'s tail).
pub const ACTION_FIRST_STOP: u8 = 0x04;

/// The amount the game writes for a "Max" order - not a quantity, a "take what fits", so
/// it must never be summed as capacity.
pub const AMOUNT_MAX: i32 = 1_000_000_000;

/// Raw units per unit of ship capacity. Every ware is stored in raw units - 200 to the
/// barrel, 2000 to the load - and ship capacity is quoted in barrels, so `raw / 200`
/// converts either kind into the number the panel shows.
pub const RAW_PER_CAPACITY: i32 = 200;

pub const WARE_SLOTS: usize = 24;

/// One route stop in the pool.
#[derive(Clone, Debug, Copy)]
pub struct RouteStopPtr {
    pub address: u32,
}

impl RouteStopPtr {
    /// The stop at `index` in the pool, or `None` when the index is outside it.
    pub unsafe fn from_pool_index(index: u16) -> Option<Self> {
        let base = *ROUTE_STOP_POOL_PTR_ADDRESS;
        if base == 0 || index >= *ROUTE_STOP_POOL_SIZE_ADDRESS {
            return None;
        }
        Some(Self {
            address: base + index as u32 * ROUTE_STOP_SIZE,
        })
    }

    /// The pool index of the next stop, from the two bytes a `.rou` file leaves unused.
    pub unsafe fn get_next_index(&self) -> u16 {
        self.get(0x00)
    }

    pub unsafe fn get_town_index(&self) -> u8 {
        self.get(0x02)
    }

    /// Repair setting plus [ACTION_FIRST_STOP].
    pub unsafe fn get_action(&self) -> u8 {
        self.get(0x03)
    }

    pub unsafe fn is_first_stop(&self) -> bool {
        self.get_action() & ACTION_FIRST_STOP != 0
    }

    /// Price and amount for one ware slot. The pair encodes the direction:
    ///
    /// |Price|Amount|Direction|
    /// |-|-|-|
    /// |`0`|negative|ship -> office (unload)|
    /// |`0`|positive|office -> ship (**load**)|
    /// |positive|positive|ship -> town (sell)|
    /// |negative|positive|town -> ship (**buy**)|
    pub unsafe fn get_ware_order(&self, slot: usize) -> Option<(i32, i32)> {
        if slot >= WARE_SLOTS {
            return None;
        }
        let price: i32 = self.get(0x1C + slot as u32 * 4);
        let amount: i32 = self.get(0x7C + slot as u32 * 4);
        Some((price, amount))
    }

    /// How much hold this stop's inbound orders ask for, in units of ship capacity, and
    /// whether any of them is a "Max" order.
    ///
    /// Inbound means **`amount > 0` and `price <= 0`** - an office load or a town
    /// purchase. A positive amount with a positive price is a *sale*, which frees hold
    /// rather than filling it, so amount alone is not the test.
    ///
    /// A [AMOUNT_MAX] order has no finite requirement - it fills whatever is left - so it
    /// is excluded from the sum and reported through the flag instead.
    pub unsafe fn load_capacity(&self) -> (i32, bool) {
        let mut capacity = 0i32;
        let mut has_max = false;
        for slot in 0..WARE_SLOTS {
            let Some((price, amount)) = self.get_ware_order(slot) else { continue };
            if amount <= 0 || price > 0 {
                continue;
            }
            if amount >= AMOUNT_MAX {
                has_max = true;
                continue;
            }
            capacity = capacity.saturating_add(amount / RAW_PER_CAPACITY);
        }
        (capacity, has_max)
    }
}

impl P3Pointer for RouteStopPtr {
    fn get_address(&self) -> u32 {
        self.address
    }
}

/// The logical first stop of the route the ship at `current_stop_index` is running.
///
/// Walks the circular chain looking for [ACTION_FIRST_STOP], bounded by the pool size so
/// a corrupt or self-referential chain cannot spin. `None` when the ship has no route, or
/// when no stop in the chain claims to be the first.
pub unsafe fn find_first_stop(current_stop_index: u16) -> Option<RouteStopPtr> {
    let limit = *ROUTE_STOP_POOL_SIZE_ADDRESS as usize;
    let mut index = current_stop_index;
    for _ in 0..limit.max(1) {
        let stop = RouteStopPtr::from_pool_index(index)?;
        if stop.is_first_stop() {
            return Some(stop);
        }
        let next = stop.get_next_index();
        if next == index {
            return None;
        }
        index = next;
    }
    None
}
