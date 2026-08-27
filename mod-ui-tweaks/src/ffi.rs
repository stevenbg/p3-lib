//! Assorted quality-of-life tweaks. Current features:
//!
//! **Extra speed** - numpad `*` / numpad `/` scale the game's time x1 / x2 / x4 /
//! x8, anywhere: world map, town view, sea battle. They sit next to the game's own
//! speed-slider keys (numpad `+` / `-`) without colliding with them: the game's key
//! dispatch at `0x00424B9B` handles only numpad `+`, numpad `-`, Pause and Tab, so
//! `*` and `/` are free, and no modifier is needed. The intended use
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

use std::{
    ffi::c_void,
    sync::atomic::{AtomicPtr, AtomicU32, Ordering},
};

use hooklet::windows::x86::{deploy_rel32_raw, X86Rel32Type};
use log::{error, info, warn};
use p3_api::{
    hotkeys::HotkeysApi,
    ui::ui_notifications::UINotificationsPtr,
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

/// Post an in-game popup on the event ticker (the top-left boxes), mirrored to the
/// debug log. The ticker renders over the local map too.
unsafe fn notify(text: &str) {
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
        }
        Err(reason) => warn!("hotkeys registry unavailable ({reason}) - the speed keys are inert"),
    }

    info!("loaded: numpad * / numpad / scale time x1/x2/x4/x8");
    0
}

/// Step the scale through 1 / 2 / 4 / 8: numpad `*` up, numpad `/` down. Declines,
/// so the game still sees the keys (its own dispatch handles only numpad +, numpad
/// -, Pause and Tab, so there is nothing to collide with either way).
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
