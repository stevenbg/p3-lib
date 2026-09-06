use crate::data::p3_ptr::P3Pointer;

/// The local-map scene window: ONE window object serves both the town view and the
/// sea battle - what differs is the map loaded into it. Constructed once at startup
/// (`0x00424DD4`, 0xCBA8 bytes, ctor `0x00586FF0`), pointer kept in the static
/// below, vtable at `0x00677998` (a second interface vtable `0x00677990` sits at
/// object `+0x94`; the main table ends around `+0xF8` - no open/close slots).
///
/// The per-frame update `+0xF4` = `0x0058B7F0` drives the whole scene frame
/// (simulation, battle AI, wind `0x006113C9`, the changed-rect submit `0x004B9650`)
/// and paces the battle simulation by the frame clock (`0x006DCCF8`), not by the
/// game tick and not by how often it is called.
pub const LOCAL_MAP_WINDOW_PTR_ADDRESS: u32 = 0x006E51AC;

#[derive(Clone, Debug, Copy)]
pub struct UILocalMapWindowPtr {
    pub address: u32,
}

impl UILocalMapWindowPtr {
    /// The module-relative offset of the vtable, for `hook_function_pointer`.
    pub const VTABLE_OFFSET: u32 = 0x277998;

    /// The current window object, or `None` before the game has built its windows.
    pub fn new() -> Option<Self> {
        let address = unsafe { *(LOCAL_MAP_WINDOW_PTR_ADDRESS as *const u32) };
        if (0x0001_0000..0x7fff_0000).contains(&address) {
            Some(Self { address })
        } else {
            None
        }
    }

    /// The loaded map's id. Two loaders store it: `0x0058A733` writes the id as
    /// given, `0x0058A395` sets bit `0x80` first (`or al,0x80`); the low byte reads
    /// `0xFF` when no map is loaded (`0x00589DEE`, `0x0058B590`). The scene's own
    /// update starts with `and eax,0x7F` and a compare against the town count.
    ///
    /// The id space is only partly mapped, and "is this a battle" is NOT decidable
    /// from it: towns are also attacked from the sea on their own maps. The map
    /// files (`iso/towns/<id>.*`) come as ids 0..30 (the towns), 128..155 and
    /// 201..205 (five of them - `SeaBattleShaderCnt=5` in `scripts/iso.ini`) plus
    /// 251..255; which class means what has not been pinned down.
    pub fn get_map_id(&self) -> u32 {
        unsafe { self.get(0xc324) }
    }

    /// The town whose map is loaded (the low 7 bits - what the scene's own update
    /// extracts and compares against the town count), or `None` when they do not
    /// name a town. Note a value below the town count does not by itself mean a
    /// peaceful town visit; see [`Self::get_map_id`].
    pub fn get_town_index(&self) -> Option<u8> {
        let index = (self.get_map_id() & 0x7f) as u8;
        let towns_count = unsafe { *(0x006de4b0 as *const u32) } as u8;
        if index < towns_count {
            Some(index)
        } else {
            None
        }
    }

    /// Is the scene on screen? The loaded map stays in the object when the player leaves
    /// for the world map - only the window is hidden (`+0xCC(0)` at `0x0042D64D`,
    /// `0x0046D5E7`) and shown again on entering (`+0xCC(1)` at `0x0044A1E4`) - so the map
    /// id alone does not say whether the player is looking at the town.
    pub unsafe fn is_shown(&self) -> bool {
        crate::ui::widget::is_visible(self.address)
    }

    /// Load a town into the scene - what the scrollmap does when the player double-clicks
    /// a town he may enter (`0x0044A1B7`), and what the ship overview does to jump to a
    /// ship's town (`0x00476B2E`). `thiscall(this, town, reload, announce, keep_panel,
    /// centre)`, `ret 0x14`, [Self::ENTER_TOWN_ADDRESS]:
    ///
    /// - the same town with `reload` = 0 only recentres the view on the town's map
    ///   position (`0x0058A5C7`..`0x0058A666`); any other town, or `reload` != 0, releases
    ///   the loaded map (`0x00589130`), stores the id at `+0xC324` (`0x0058A733`) and loads
    ///   the new one (`0x0058EB30`), so it works from inside another town as well as from
    ///   the world map;
    /// - `announce` != 0 tells the notification manager `[0x006CBB40]` about the town
    ///   (`0x0042CD90`) and runs `0x0058F4E0(1)`; `keep_panel` = 0 additionally calls its
    ///   `0x0042A6C0`. The scrollmap passes `(town, 0, 1, 1, 1)` and then hides itself
    ///   (`+0xCC(0)`); the ship overview passes `(town, 0, 0, flag, 1)`.
    ///
    /// The session start enters the home town with `(town, 1, 0, 1, 1)` (`0x004339A1`) -
    /// `reload` set because the scene is coming from the world map, where the map kept in
    /// the object is not the one on screen.
    ///
    /// `keep_panel` and `centre` are passed as 1. The wrapper does not check whether the
    /// player may enter the town - see
    /// [crate::game_world::GameWorldPtr::can_merchant_enter_town] - and it does not close
    /// windows open in the current town or swap the scenes.
    pub unsafe fn enter_town(&self, town_index: u8, reload: bool, announce: bool) {
        let enter: extern "thiscall" fn(this: u32, town: u32, reload: u32, announce: u32, keep_panel: u32, centre: u32) =
            std::mem::transmute(Self::ENTER_TOWN_ADDRESS);
        enter(self.address, town_index as u32, reload as u32, announce as u32, 1, 1);
    }
}

impl UILocalMapWindowPtr {
    /// See [Self::enter_town].
    pub const ENTER_TOWN_ADDRESS: u32 = 0x0058A4F0;
}

impl P3Pointer for UILocalMapWindowPtr {
    fn get_address(&self) -> u32 {
        self.address
    }
}
