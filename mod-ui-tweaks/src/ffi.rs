//! Assorted quality-of-life tweaks. Current features:
//!
//! **Extra speed** - numpad `*` / numpad `/` scale the game's time x1 / x2 / x4 /
//! x8, anywhere: world map, town view, sea battle. They sit next to the game's own
//! speed-slider keys (numpad `+` / `-`) without colliding with them: the dispatcher
//! owning the speed-slider keys (`0x00424B9B`) handles only numpad `+`, numpad `-`,
//! Pause and Tab - it is one dispatcher among several, but no handler for `*` or
//! `/` shows up in a full-exe scan - so no modifier is needed. The intended use
//! is speeding up sea battles, but battles are not a map type of their own - a town
//! attacked from the sea fights on the town's map - so the keys are deliberately
//! global instead of scene-scoped.
//!
//! Why time itself: the local-map simulation (ships, projectiles, battle AI) is
//! neither game-tick-paced nor call-count-paced - its per-frame update measures
//! elapsed real time off the frame clock and advances by that, verified by calling
//! it eight times per frame to no effect. The frame clock updater (`0x004BD180`,
//! once per frame from the main loop, the game's 50 fps limiter inside) computes
//! this frame's real elapsed ms and both stores it to the frame-delta global
//! (`[0x6DCCF4]`) and accumulates it into the game clock (`[0x6DCCF8]`). This mod
//! detours the 5-byte delta computation (`0x004BD1E2`) to multiply the delta first,
//! so everything paced by either global - local-map simulation and the world's tick
//! pacer alike - sees dilated time.
//!
//! The keys are dispatched through the shared hotkey registry (`hotkeys.dll`, see
//! mod-hotkeys), registered session-globally; without the registry they are inert.
//!
//! **Pirate ship attacks do not interrupt fast forward** - a notorious pirate robbing
//! someone else's ship raises "<name> has struck again", which plays its video (or posts a
//! ticker message) and drops the game back to normal speed. It is frequent enough to make
//! fast forward unusable. While the game is **in fast forward** this mod suppresses that
//! one event outright: no video, no message, no speed change. At any other speed it is
//! left completely alone, so nothing is lost when the player is actually watching.
//!
//! It is suppressed at its single call site (`0x0060EBA7`) rather than further down,
//! because that is the only point upstream of both the video and the ticker - see
//! `p3_api::ui::ui_event_window::PIRATE_STRUCK_AGAIN_CALL_OFFSET`.
//!
//! **Late auction announcement** - vanilla announces "Tomorrow there will be an auction
//! in %s." at midnight, a full day before the auction, which also runs at midnight. That
//! is long enough to forget about. This moves the announcement to late in the same day,
//! leaving the auction itself exactly where it was - see [AUCTION_ANNOUNCE_TIME_OF_DAY].

use std::{
    ffi::c_void,
    sync::atomic::{AtomicPtr, AtomicU32, Ordering},
};

use hooklet::windows::x86::{deploy_rel32_raw, X86Rel32Type};
use log::{error, info, warn};
use p3_api::{
    hotkeys::{HotkeysApi, MOD_ALT, MOD_CTRL},
    memory::write_readonly,
    operations::GAME_SPEED_LEVEL_ADDRESS,
    ui::{
        ui_event_window::{
            PIRATE_EVENT_REGION_ENTRY, PIRATE_EVENT_REGION_ENTRY_BYTES, STATIC_UI_EVENT_WINDOW_PTR,
        },
        ui_notifications::UINotificationsPtr,
    },
};
use windows::Win32::UI::Input::KeyboardAndMouse::{VK_DIVIDE, VK_MULTIPLY};

/// The frame clock updater's delta computation: `sub eax,edx; pop edi; add ecx,eax`
/// (eax = this frame's real elapsed ms, ecx = the accumulating game clock) - 5 bytes,
/// an exact jmp fit. The two stores right after the continuation
/// (`[0x6DCCF4] = eax`, `[0x6DCCF8] = ecx`) then see the scaled delta.
const CLOCK_DELTA_PATCH_ADDRESS: u32 = 0x004bd1e2;
static CLOCK_DELTA_CONTINUATION: u32 = 0x004bd1e7;
const CLOCK_DELTA_ORIGINAL: [u8; 5] = [0x2b, 0xc2, 0x5f, 0x03, 0xc8];

const MAX_SCALE: u32 = 8;

/// The time multiplier the detour applies. Read by the assembly stub every frame.
static SCALE: AtomicU32 = AtomicU32::new(1);
/// The shared hotkey registry (hotkeys.dll); null = unavailable, the speed keys are
/// inert. Dispatch used to be a per-frame GetKeyState poll off the clock updater's
/// two call sites; the registry's keyboard hook replaces all of it.
static HOTKEYS: AtomicPtr<HotkeysApi> = AtomicPtr::new(std::ptr::null_mut());
const OWNER: &std::ffi::CStr = c"ui-tweaks";

/// Where [pirate_event_detour] resumes when it lets the announcement run: past all six
/// bytes of the replaced instruction. Read indirectly by the stub.
static PIRATE_EVENT_CONTINUE: u32 = 0x0060EB5B;
/// Where [pirate_event_detour] jumps to skip the announcement: the region's single exit.
static PIRATE_EVENT_EXIT: u32 = 0x0060EBB6;

/// When the auction announcement fires, as a **time of day in 1/256 of a day** - the unit
/// the scheduled-task due times and the game clock (`[0x006DE4B4]`) both use.
///
/// `0x00` is vanilla: midnight, a full 24 h before the auction. `0xF0` is 22:30, i.e.
/// **1.5 h of warning**. `0xE0` = 21:00 (3 h), `0xC0` = 18:00 (6 h), `0x80` = noon (12 h).
///
/// The auction itself does not move whatever this is set to: the announcement state adds a
/// whole day and re-aligns the low byte to zero, discarding the offset, so the auction
/// stays at midnight of the following day.
///
/// Going much beyond `0xF0` leaves too little time to reach the town - the value is the
/// only thing to change here, so retuning is a rebuild and nothing else.
const AUCTION_ANNOUNCE_TIME_OF_DAY: u8 = 0xF0;

/// The two `mov byte [esi],0` instructions that pin the auction **announcement** to
/// midnight, inside the auction scheduled task (kind `0x0A`, handler `0x004E2CD4`):
/// `0x004E2DFC` schedules it, `0x004E2EFB` re-schedules when the town already has an
/// auction running and this one slips a day. Both must move together or a slipped auction
/// reverts to announcing at midnight.
///
/// Deliberately **not** `0x004E2FDA`, the third instance - that one sets the auction's own
/// time, and moving it would drag the auction along and change nothing about the gap.
const AUCTION_ANNOUNCE_SITES: [u32; 2] = [0x004E2DFC, 0x004E2EFB];
/// `mov byte [esi],0` - the immediate is the third byte.
const AUCTION_ANNOUNCE_ORIGINAL: [u8; 3] = [0xc6, 0x06, 0x00];

/// Post an in-game popup on the event ticker (the top-left boxes), mirrored to the
/// debug log. The ticker renders over the local map too.
pub(crate) unsafe fn notify(text: &str) {
    info!("{text}");
    let latin1: Vec<u8> = text.chars().map(|c| if (c as u32) <= 0xff { c as u32 as u8 } else { b'?' }).collect();
    UINotificationsPtr::new().post_event(&latin1);
}

#[no_mangle]
pub unsafe extern "C" fn start() -> u32 {
    let _ = log::set_logger(&win_dbg_logger::DEBUGGER_LOGGER);
    log::set_max_level(log::LevelFilter::Info);

    let current = *(CLOCK_DELTA_PATCH_ADDRESS as *const [u8; 5]);
    if current != CLOCK_DELTA_ORIGINAL {
        error!("unexpected bytes at the clock delta ({CLOCK_DELTA_PATCH_ADDRESS:#x}): {current:02x?}");
        return 1;
    }
    if deploy_rel32_raw(
        CLOCK_DELTA_PATCH_ADDRESS as _,
        (&clock_delta_detour) as *const _ as _,
        X86Rel32Type::Jump,
    )
    .is_err()
    {
        error!("failed to detour the clock delta");
        return 2;
    }

    match HotkeysApi::bind() {
        Ok(api) => {
            HOTKEYS.store(Box::into_raw(Box::new(api)), Ordering::SeqCst);
            let api = &*HOTKEYS.load(Ordering::SeqCst);
            // Session-global, so the handles are never stored.
            api.register(OWNER, VK_MULTIPLY.0 as u32, 0, speed_hotkeys);
            api.register(OWNER, VK_DIVIDE.0 as u32, 0, speed_hotkeys);
            api.register(OWNER, b'S' as u32, MOD_CTRL, crate::sailors::hire_sailors_hotkey);
            api.register(OWNER, b'S' as u32, MOD_ALT, crate::sailors::hire_sailors_hotkey);
            api.register(OWNER, b'S' as u32, MOD_CTRL | MOD_ALT, crate::sailors::hire_sailors_hotkey);
        }
        Err(reason) => warn!("hotkeys registry unavailable ({reason}) - the speed keys are inert"),
    }

    for site in AUCTION_ANNOUNCE_SITES {
        let found = *(site as *const [u8; 3]);
        if found != AUCTION_ANNOUNCE_ORIGINAL {
            error!("unexpected bytes at the auction schedule {site:#010x}: {found:02x?} - not patching");
            return 4;
        }
    }
    for site in AUCTION_ANNOUNCE_SITES {
        if let Err(reason) = write_readonly(site + 2, &[AUCTION_ANNOUNCE_TIME_OF_DAY]) {
            error!("failed to move the auction announcement at {site:#010x}: {reason}");
            return 5;
        }
    }

    let found = *(PIRATE_EVENT_REGION_ENTRY as *const [u8; 6]);
    if found != PIRATE_EVENT_REGION_ENTRY_BYTES {
        error!("unexpected bytes at the pirate event {PIRATE_EVENT_REGION_ENTRY:#010x}: {found:02x?} - not patching");
        return 3;
    }
    if deploy_rel32_raw(
        PIRATE_EVENT_REGION_ENTRY as _,
        (&pirate_event_detour) as *const _ as _,
        X86Rel32Type::Jump,
    )
    .is_err()
    {
        error!("failed to detour the pirate event at {PIRATE_EVENT_REGION_ENTRY:#010x}");
        return 6;
    }

    info!(
        "loaded: numpad * / numpad / scale time x1/x2/x4/x8; pirate attacks are suppressed while in fast forward; auction announced at {:#04x}/256 of the day",
        AUCTION_ANNOUNCE_TIME_OF_DAY
    );
    0
}

/// Step the scale through 1 / 2 / 4 / 8: numpad `*` up, numpad `/` down. Declines,
/// so the game still sees the keys (no game handler for them is known).
#[no_mangle]
unsafe extern "C" fn speed_hotkeys(vk: u32, _mods: u32) -> u32 {
    let scale = SCALE.load(Ordering::Relaxed);
    let new = if vk == VK_MULTIPLY.0 as u32 { (scale * 2).min(MAX_SCALE) } else { (scale / 2).max(1) };
    if new != scale {
        SCALE.store(new, Ordering::Relaxed);
        notify(&format!("extra speed: x{new}"));
    }
    0
}

extern "C" {
    static clock_delta_detour: c_void;
    static pirate_event_detour: c_void;
}

// Replaces `sub eax,edx; pop edi; add ecx,eax` at 0x4BD1E2: eax = the raw delta ms,
// ecx = the accumulating game clock. Scale the delta before both consumers see it.
std::arch::global_asm!("
.global {detour}
{detour}:
sub eax, edx
imul eax, dword ptr [{scale}]
pop edi
add ecx, eax
jmp [{continuation}]
",
    detour = sym clock_delta_detour,
    scale = sym SCALE,
    continuation = sym CLOCK_DELTA_CONTINUATION,
);

// Replaces `mov ecx,[0x006CC7E8]` at 0x0060EB55, the single entry to the "<pirate> has
// struck again" announcement. In fast forward, jump to the region's single exit so none of
// it happens - the same exit the game itself takes when the robbed merchant is the player.
// Otherwise perform the replaced instruction and carry on.
//
// Only the flags are clobbered, and they are dead here: the `je` two instructions above
// already consumed them and the next thing in the region is a call. The replaced
// instruction loads ecx itself, so nothing else needs preserving.
std::arch::global_asm!("
.global {detour}
{detour}:
cmp dword ptr [{speed_level}], {fast_forward}
je 2f
mov ecx, dword ptr [{event_window}]
jmp [{continuation}]
2:
jmp [{exit}]
",
    detour = sym pirate_event_detour,
    speed_level = const GAME_SPEED_LEVEL_ADDRESS,
    fast_forward = const p3_api::operations::FAST_FORWARD_LEVEL,
    event_window = const STATIC_UI_EVENT_WINDOW_PTR,
    continuation = sym PIRATE_EVENT_CONTINUE,
    exit = sym PIRATE_EVENT_EXIT,
);
