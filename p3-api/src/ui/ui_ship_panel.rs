use crate::{data::p3_ptr::P3Pointer, memory::is_readable, ship::ShipPtr, ships::ShipsPtr};

pub const STATIC_UI_SHIP_PANEL_PTR_ADDRESS: *const u32 = 0x006CE6D0 as _;

/// The scrollmap's right-side panel for the selected ship or convoy. Its four buttons
/// switch the view - Goods, Crew, Deck, Auto trade - so the trade route is only one of
/// the things it shows; the selection below is common to all four.
/// Vtable `0x0066F358`, per-frame update `+0xF4` = `0x0048B3E0`.
///
/// Constructed once at startup like the building windows, but its static sits apart
/// from the `0x006E55xx` window cluster: the mass-constructor allocates `0x83C8` bytes
/// at `0x004266D2`, calls the constructor (`0x00486D20`) at `0x004266E7` and stores the
/// result into the static at `0x00426700`. Reading the static is therefore enough - no
/// vtable hook needed to obtain the object.
#[derive(Clone, Debug, Copy)]
pub struct UIShipPanelPtr {
    pub address: u32,
}

impl Default for UIShipPanelPtr {
    fn default() -> Self {
        Self::new()
    }
}

impl UIShipPanelPtr {
    pub const VTABLE_OFFSET: u32 = 0x26F358;
    /// The panel's own code reads the selection through this field (`0x0048C363`, and
    /// the route Load handler at `0x0048C92E`), which is what makes it authoritative
    /// for "which ship is selected": it holds on the world map and in town, for own
    /// and foreign ships alike.
    pub const SELECTION_OFFSET: u32 = 0xA0;

    pub fn new() -> Self {
        Self {
            address: unsafe { *STATIC_UI_SHIP_PANEL_PTR_ADDRESS },
        }
    }

    /// The current selection object, whose first `u16` is the selected ship index.
    /// `None` before the panel exists or while nothing is selected.
    ///
    /// Observed: the field holds `0` whenever nothing is selected - it is cleared, not
    /// left pointing at the previous selection - and selecting a convoy on the map
    /// reports its leader ship, so the value is always a ship index and never a convoy
    /// one. **Opening any building window clears it**, and closing the window selects
    /// the same ship again, so a mod cannot read the selection while a building window
    /// is on screen.
    ///
    /// The selection object is heap-allocated and freed when the selection changes (the
    /// setter at `0x00487E74` frees the old one before storing the new pointer), so the
    /// readability check is what keeps a dangling read from crashing.
    ///
    /// Only the leading `u16` is written: the bytes after it are uninitialised, and
    /// re-selecting one ship yields a different value each time - values that match the
    /// high half of neighbouring heap pointers. Do not read past the index.
    pub unsafe fn get_selection(&self) -> Option<u32> {
        if !is_readable(self.address + Self::SELECTION_OFFSET, 4) {
            return None;
        }
        let selection: u32 = self.get(Self::SELECTION_OFFSET);
        if is_readable(selection, 2) {
            Some(selection)
        } else {
            None
        }
    }

    /// The index of the ship selected on the map, if the selection is a ship the ships
    /// array actually holds.
    pub unsafe fn get_selected_ship_index(&self) -> Option<u16> {
        let index = *(self.get_selection()? as *const u16);
        if index < ShipsPtr::new().get_ships_size() {
            Some(index)
        } else {
            None
        }
    }

    pub unsafe fn get_selected_ship(&self) -> Option<ShipPtr> {
        ShipsPtr::new().get_ship(self.get_selected_ship_index()?)
    }
}

impl P3Pointer for UIShipPanelPtr {
    fn get_address(&self) -> u32 {
        self.address
    }
}
