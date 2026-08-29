//! The global settings object at `[0x006CC3E8]`.
//!
//! Its low bytes hold one per option from the **"Game settings"** screen. The difficulty
//! preset is only a shortcut: `0x00463B20` writes the same value into every one of them,
//! while the custom screen (`0x004992F0`) stores each dropdown separately, decremented
//! because the UI positions are 1-based.
//!
//! The same object also carries the **Options** screen (`[OPTIONS]` in `P2.cfg`), applied
//! at `0x0045DFB3`-`0x0045E068` from the options window's own fields - see
//! [EVENT_VIDEOS_OFFSET]. Those live at `+0x1F`..`+0x26`, clear of the `+0x09`..`+0x18`
//! range the difficulty preset sweeps, so starting a new game does not reset them.

/// The settings object. Uninitialised before a game is started.
pub const GAME_SETUP_PTR_ADDRESS: *const u32 = 0x006CC3E8 as _;

/// The session-mode byte: `0`/`1`/`2` while the multiplayer mode is being chosen (written
/// beside the operation queue's status word at `0x0043A8E6`, `0x0043C4A4` and friends),
/// `5` once a game is actually running (`0x00430999`) and `4` at `0x004ABF35`.
///
/// Read as `>= 3` all over the UI to mean **"a game session is live"**, which is what
/// gates the event window's game-speed reset - see
/// [crate::ui::ui_event_window::UIEventWindowPtr].
///
/// The difficulty preset's blanket sweep writes this byte too (`0x00463B3B`), but game
/// start overwrites it, so the transient `0`..`2` is never observed in play.
pub const SESSION_MODE_OFFSET: u32 = 0xD;

/// **Event videos**, `0` off / `1` on - the "Event videos" row of the Options screen,
/// persisted as `[OPTIONS] VIDEOS` in `P2.cfg` (loaded at `0x0045D026` with default `1`
/// and clamped to `0..1`, saved at `0x0045E1F3`). Not to be confused with the "Videos"
/// row right above it, which is the video *volume* (`LSVIDEO`, `+0x22`).
///
/// The event window reads it at `0x00469B3A` to choose between playing the event's video
/// and posting a plain ticker message.
pub const EVENT_VIDEOS_OFFSET: u32 = 0x26;

/// `None` before a game is loaded.
pub fn event_videos_enabled() -> Option<bool> {
    unsafe {
        let setup = *GAME_SETUP_PTR_ADDRESS;
        if setup == 0 {
            return None;
        }
        Some(*((setup + EVENT_VIDEOS_OFFSET) as *const u8) != 0)
    }
}

/// `None` before a game is loaded. `>= 3` means a game session is live.
pub fn session_mode() -> Option<u8> {
    unsafe {
        let setup = *GAME_SETUP_PTR_ADDRESS;
        if setup == 0 {
            return None;
        }
        Some(*((setup + SESSION_MODE_OFFSET) as *const u8))
    }
}

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
