//! The new-game setup object at `[0x006CC3E8]`, which holds one byte per option from the
//! "Game settings" screen. The difficulty preset is only a shortcut: `0x00463B20` writes
//! the same value into every one of these bytes, while the custom screen (`0x004992F0`)
//! stores each dropdown separately, decremented because the UI positions are 1-based.

/// The setup object. Uninitialised before a game is started.
pub const GAME_SETUP_PTR_ADDRESS: *const u32 = 0x006CC3E8 as _;

/// **Pirates activity**, `0` low / `1` normal / `2` high - the "Pirates activity" row of
/// the settings screen (window field `+0x1B90`). All fifteen readers are pirate code or
/// world generation, so this byte alone drives pirate behaviour; the difficulty preset
/// merely writes it along with everything else.
pub const PIRATE_ACTIVITY_OFFSET: u32 = 0x13;

/// `None` before a game is loaded.
pub fn get_pirate_activity() -> Option<u8> {
    unsafe {
        let setup = *GAME_SETUP_PTR_ADDRESS;
        if setup == 0 {
            return None;
        }
        Some(*((setup + PIRATE_ACTIVITY_OFFSET) as *const u8))
    }
}

/// How many roaming pirate bands the world was generated with: `2 * activity + 1`, so
/// **1, 3 or 5**. Fixed at world generation (`0x0054A480`, called from `0x004309D2`);
/// changing the setting afterwards does not create or destroy bands, it only moves the
/// attack threshold below.
pub fn pirate_band_count() -> Option<u32> {
    get_pirate_activity().map(|a| 2 * a as u32 + 1)
}

/// The lowest home-town rank at which a free pirate will rob a merchant.
///
/// The eligibility test (`0x0051543B`) is `rank_in_home_town + pirate_activity >= 2`, so
/// the threshold is `2 - activity`: rank 2 on low, rank 1 on normal, and 0 - i.e.
/// everyone - on high. It gates the branch taken when the prey's control word is zero,
/// which is the **human** one (`0x004DCF82` builds its `is_human` flag as
/// `control_word == 0`, and that flag selects the captain scan's player branch), so this
/// is the player's own threshold. Being established is what makes you worth robbing.
pub fn pirate_attack_rank_threshold() -> Option<u8> {
    get_pirate_activity().map(|a| 2u8.saturating_sub(a))
}
