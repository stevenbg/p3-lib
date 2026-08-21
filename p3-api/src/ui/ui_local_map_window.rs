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
}

impl P3Pointer for UILocalMapWindowPtr {
    fn get_address(&self) -> u32 {
        self.address
    }
}
