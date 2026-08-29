//! The **event window**: the singleton at `[0x006CC7E8]` that announces world events -
//! the ones with a video in `Videos\` and a matching entry in the ticker.
//!
//! One entry point, `0x00469AF0`, `thiscall(this, type, a, b, text1, text2)`, called from
//! 26 sites across the game logic with a type from the table below. What it does with the
//! event depends on the **Event videos** option
//! ([crate::game_setup::EVENT_VIDEOS_OFFSET]), read at `0x00469B3A` into
//! [PLAY_VIDEO_OFFSET]:
//!
//! - **videos on**: branch to `0x0046A2A2` and play the event's video.
//! - **videos off**: format the type's message, post it on the scrollmap ticker
//!   (`0x0042B4C0` on [crate::ui::ui_notifications::STATIC_UI_NOTIFICATIONS_PTR_ADDRESS]),
//!   and then **drop the game out of fast forward** - see [SPEED_RESET_CALL_ADDRESS].
//!
//! **The message tables are NOT indexed by the event type.** Two of them exist - plain
//! text at `0x006A3BD0` and the ticker's colour-coded `\c\f4_` variants at `0x006A3808` -
//! but the type reaches its message through the jump table at `0x0046AB50`, and each case
//! loads its own string pointer. The two orders agree only up to `0x02` and diverge from
//! `0x03` on: type `0x08` is "Discovery of new transhipment location" while table slot
//! `0x08` is "Snaikka finished". Reading a type's meaning off the table gives an answer
//! that is plausible, self-consistent and wrong - it cost a shipped mod suppressing the
//! wrong event on 29 Aug 2026. Decode the case, not the table.
//!
//! `0x006A3C2C` "Pirates are attacking %s" has **no loader at all** in either table - a
//! dead string with no event type behind it.
//!
//! Not to be confused with the **letter** events funnelled through `0x00548CA0`, which
//! have their own 0x00..=0x2A type space (`0x006B28A8`) and their own way of stopping the
//! clock. Both systems can fire for one game event.

/// The event window. Uninitialised before a game is started.
pub const STATIC_UI_EVENT_WINDOW_PTR_ADDRESS: *const u32 = 0x006CC7E8 as _;
/// [STATIC_UI_EVENT_WINDOW_PTR_ADDRESS] as an integer, for assembly operands.
pub const STATIC_UI_EVENT_WINDOW_PTR: u32 = 0x006CC7E8;

/// `thiscall(this, type, a, b, text1, text2)` - announce an event.
pub const SHOW_EVENT_ADDRESS: u32 = 0x00469AF0;

/// The event type the window is showing - but **only on the video path**.
///
/// `0x00469BAD` writes it only when `+0x3EA` is zero. On the text path (event videos off)
/// the store is skipped and the field keeps its idle `-1`, measured 29 Aug 2026 as
/// `type=0xffffffff` on every event. Do not use this to identify an event whose message
/// went to the ticker - the type is live in `ebx` from `0x00469B5C` to the end of the
/// function instead, and `ebx` is callee-saved.
///
/// [current_event_type] therefore returns `None` for exactly the events this module's own
/// consumer cares about; it is kept for the video path and for completeness.
pub const EVENT_TYPE_OFFSET: u32 = 0x3C0;

/// Set from [crate::game_setup::EVENT_VIDEOS_OFFSET] at `0x00469B3A`: non-zero means this
/// event plays its video instead of posting a ticker message.
pub const PLAY_VIDEO_OFFSET: u32 = 0x380;

/// The `call 0x0054AA70` at `0x0046A26C` that **ends fast forward**.
///
/// It sits at the end of the text path (`0x0046A1FA`), right after the ticker message is
/// posted, and enqueues [crate::operation::Operation::SetGameSpeed] with both divisors and
/// `field_10` left at `-1` and `level` forced to `0` - normal play. Guarded only by
/// `cmp byte [settings+0xD], 3 / jb` at `0x0046A22E`, i.e. "is a game session live", so
/// **every** event type that reaches the text path stops the clock, with nothing to
/// distinguish a new ship from a pirate raid.
///
/// Module-relative for `hooklet`'s `hook_call_rel32`, which adds the module base itself.
pub const SPEED_RESET_CALL_OFFSET: u32 = 0x0006A26C;
/// Absolute form of [SPEED_RESET_CALL_OFFSET].
pub const SPEED_RESET_CALL_ADDRESS: u32 = 0x0046A26C;

/// The whole "<pirate> has struck again" announcement, as one skippable region:
/// `0x0060EB55` (entry) to `0x0060EBB6` (exit).
///
/// ```text
/// 60eb53:  je   0x60ebb6        ; the victim is the player -> vanilla skips it already
/// 60eb55:  mov  ecx,[0x006CC7E8]   <-- the only way in
/// 60eb5d:  call 0x00469380         ; open and SHOW the window
/// 60eba7:  call 0x00469AF0         ; populate it
/// 60ebac:  mov  eax,[0x006DE4B4]
/// 60ebb1:  mov  [0x006E59D8],eax   ; stamp when the last event fired
/// 60ebb6:  ...                     <-- the only way out
/// ```
///
/// **The two calls must be suppressed together or not at all.** `0x00469380` shows the
/// window and `0x00469AF0` fills it in; skip only the second and the window is displayed
/// with [EVENT_TYPE_OFFSET] still at its construction `-1`, whereupon the render loop
/// indexes `window+0x3BC` with it (`0x0046760C`, no bound check), looks up a garbage string
/// id and faults on the NULL result at `0x00467626`. Measured the hard way, 29 Aug 2026.
///
/// Skipping the region entire avoids the question, and reproduces a path the game already
/// has: the `je` at `0x0060EB53` takes the same exit when the robbed merchant is the player.
/// It also skips the `0x006E59D8` stamp, which is correct - the event did not happen.
///
/// Nothing branches into the interior; the only inbound edges are the fall-through at the
/// entry and seven branches to the exit. Verified across the whole executable.
///
/// The instruction at the entry is **6** bytes (`8b 0d e8 c7 6c 00`) and a `jmp rel32` is
/// 5, so a detour must resume at [PIRATE_EVENT_REGION_CONTINUE] - never at the orphaned
/// sixth byte.
pub const PIRATE_EVENT_REGION_ENTRY: u32 = 0x0060EB55;
/// The bytes [PIRATE_EVENT_REGION_ENTRY] replaces: `mov ecx,[0x006CC7E8]`.
pub const PIRATE_EVENT_REGION_ENTRY_BYTES: [u8; 6] = [0x8b, 0x0d, 0xe8, 0xc7, 0x6c, 0x00];
/// Where a detour resumes when it lets the announcement happen: past all 6 replaced bytes.
pub const PIRATE_EVENT_REGION_CONTINUE: u32 = 0x0060EB5B;
/// Where a detour jumps to skip the announcement: the region's single exit.
pub const PIRATE_EVENT_REGION_EXIT: u32 = 0x0060EBB6;

/// A **notorious pirate has robbed someone else's ship** - `"%s has struck again"`.
///
/// Case `0x00469FF0`: `%s` is the **pirate's name**, looked up from the event's second
/// argument through `0x00512AE0` on `[0x006DDAA0]` - so that argument is a pirate index,
/// not a town.
///
/// Raised at `0x0060EBA7` with the pirate index from `this+0x1E91`, and **only when the
/// victim is not the player**: `0x0060EB51` compares the victim's merchant index at
/// `[ebx]` against the player's (`[0x006DFC14]` = `ops+0x924`) and branches away to
/// `0x0060EBB6` when they match, so losing your own ship is announced by something else.
///
/// Frequent enough in normal play to make fast forward unusable, which is why
/// mod-ui-tweaks suppresses its speed reset.
pub const EVENT_PIRATE_STRUCK_AGAIN: u32 = 0x14;

/// A town is under siege - `"Siege in %s"`, case `0x0046A012`, `%s` being the town name.
///
/// Raised at `0x00629D36` inside `0x00629C60`, the routine `handle_start_siege`
/// (`0x00633AF0`) calls from its single site `0x00633C30`. Gated on the besieged town
/// concerning the player: it is the player's home town (`merchant+0x19`), or the player is
/// its **Lord Mayor** (`town+0x6F1`, `0xFF` = none).
///
/// Adjacent to [EVENT_PIRATE_STRUCK_AGAIN] and easy to confuse with it - the string table
/// puts "has struck again" in slot `0x15`, and this is the trap the module doc warns
/// about.
pub const EVENT_SIEGE: u32 = 0x15;

/// The type the event window is currently showing, or `None` before a game is loaded.
pub fn current_event_type() -> Option<u32> {
    unsafe {
        let window = *STATIC_UI_EVENT_WINDOW_PTR_ADDRESS;
        if window == 0 {
            return None;
        }
        let event_type = *((window + EVENT_TYPE_OFFSET) as *const i32);
        if event_type < 0 {
            return None;
        }
        Some(event_type as u32)
    }
}
