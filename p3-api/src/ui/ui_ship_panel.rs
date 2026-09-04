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
    /// Which of the panel's views is showing. The panel keeps the possible ids in the
    /// **fields** `+0xD0`..`+0xE8` - the constructor stores `0`..`6` there
    /// (`0x00486D8D`..`0x00486DB3`) - so every test in the game reads as
    /// `cmp [esi+0xCC],[esi+0xE4]` rather than against an immediate, and searching for a
    /// literal finds nothing.
    ///
    /// Measured by pressing each button in play:
    ///
    /// |Value|View|
    /// |-|-|
    /// |`0`|Goods|
    /// |`1`|Crew|
    /// |`2`|Deck|
    /// |`5`|**Auto trade**|
    ///
    /// `3`, `4` and `6` were not produced by the four buttons and are unidentified.
    /// The auto-trade id is corroborated in code: `0x0048BEA7` loads `+0xE4` and
    /// `0x0048BEAF` stores it into this field, and the draw method's auto-trade branch
    /// compares against `+0xE4` at `0x0048B1AF`.
    ///
    /// Note the neighbouring `+0xB4`, with its own id set in `+0xB8`..`+0xC8` (`0`..`4`),
    /// is a **different** state variable - it held `1` throughout a ship selection while
    /// the view changed under it. Its meaning is not established. The view switcher
    /// `0x00488150` writes the two together.
    pub const VIEW_OFFSET: u32 = 0xCC;
    /// The value [Self::VIEW_OFFSET] takes while the Goods view is showing - the default
    /// view, which draws the same barrel-and-capacity artwork as the Auto trade view.
    pub const VIEW_GOODS: u32 = 0;
    /// A third view that draws the same barrel-and-capacity artwork as the Goods and
    /// Auto trade views (confirmed in play). Not produced by the four named buttons, so
    /// its title is unidentified, but it shares the barrel layout.
    pub const VIEW_GOODS_ALT: u32 = 3;
    /// The value [Self::VIEW_OFFSET] takes while the Auto trade view is showing.
    pub const VIEW_AUTO_TRADE: u32 = 5;

    pub fn new() -> Self {
        Self {
            address: unsafe { *STATIC_UI_SHIP_PANEL_PTR_ADDRESS },
        }
    }

    /// The panel's top-left on screen, the origin its own draw method adds its offsets
    /// to (`0x0048B071`/`0x0048B074`). Panel-relative coordinates plus these follow the
    /// panel wherever the scrollmap puts it.
    pub fn get_x(&self) -> i32 {
        unsafe { self.get(0x14) }
    }

    pub fn get_y(&self) -> i32 {
        unsafe { self.get(0x18) }
    }

    /// The panel's own extent, the values its draw method adds to the origin at
    /// `0x0048B089`/`0x0048B08B` to get its bottom-right.
    pub fn get_width(&self) -> i32 {
        unsafe { self.get(0x2C) }
    }

    pub fn get_height(&self) -> i32 {
        unsafe { self.get(0x30) }
    }

    /// The panel's current view - see [Self::VIEW_OFFSET] for what the values mean.
    /// `None` before the panel exists.
    pub unsafe fn get_view(&self) -> Option<u32> {
        if !is_readable(self.address + Self::VIEW_OFFSET, 4) {
            return None;
        }
        Some(self.get(Self::VIEW_OFFSET))
    }

    /// Is the panel showing its Auto trade view? What a mod needs before drawing into it.
    pub unsafe fn is_auto_trade_view(&self) -> bool {
        self.get_view() == Some(Self::VIEW_AUTO_TRADE)
    }

    /// Is the panel on a view that draws the barrel-and-capacity artwork - Goods,
    /// Auto trade, or the third barrel view [Self::VIEW_GOODS_ALT], which share that
    /// layout? What a mod needs before drawing over the barrel.
    pub unsafe fn is_barrel_view(&self) -> bool {
        matches!(self.get_view(), Some(Self::VIEW_GOODS | Self::VIEW_GOODS_ALT | Self::VIEW_AUTO_TRADE))
    }

    /// The current selection object, whose first `u16` is the selected ship index.
    /// `None` before the panel exists or while nothing is selected.
    ///
    /// Observed: the field holds `0` whenever nothing is selected - it is cleared, not
    /// left pointing at the previous selection. The value is always a ship index and
    /// never a convoy one, but it is **not** necessarily the convoy's leader: measured
    /// against a 101/161/180/187 convoy whose leader is 161, both selecting the convoy
    /// as a whole and clicking a member reported ship 101, so a mod that needs the
    /// leader must resolve `convoy+0x10` itself.
    /// **Opening any building window clears it**, and closing the window selects
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
