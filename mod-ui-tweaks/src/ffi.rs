//! Assorted quality-of-life tweaks. Current features:
//!
//! **Extra speed** - ALT+`+` / ALT+`-` (main row only: the numpad +/- are the
//! game's own speed-slider keys, and its handler ignores modifiers, so binding the
//! numpad too made ALT+numpad-+ trigger both) scale the game's time x1 / x2 / x4 /
//! x8, anywhere: world map, town view, sea battle. The intended use
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
//! The keys are polled once per frame from the clock updater's two call sites
//! (`0x004B8AE2`, `0x004B8F7E` - hooking both covers every message loop), so no OS
//! keyboard hook is installed and other mods' keys are untouched.

use std::{
    ffi::c_void,
    mem,
    sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, Ordering},
};

use hooklet::windows::x86::{deploy_rel32_raw, hook_call_rel32, CallRel32Hook, X86Rel32Type};
use log::{error, info};
use p3_api::ui::ui_notifications::UINotificationsPtr;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VIRTUAL_KEY, VK_MENU, VK_OEM_MINUS, VK_OEM_PLUS};

/// The frame clock updater's delta computation: `sub eax,edx; pop edi; add ecx,eax`
/// (eax = this frame's real elapsed ms, ecx = the accumulating game clock) - 5 bytes,
/// an exact jmp fit. The two stores right after the continuation
/// (`[0x6DCCF4] = eax`, `[0x6DCCF8] = ecx`) then see the scaled delta.
const CLOCK_DELTA_PATCH_ADDRESS: u32 = 0x004bd1e2;
static CLOCK_DELTA_CONTINUATION: u32 = 0x004bd1e7;
const CLOCK_DELTA_ORIGINAL: [u8; 5] = [0x2b, 0xc2, 0x5f, 0x03, 0xc8];

/// The frame clock updater `0x004BD180` has exactly two call sites (module-relative
/// below); hooking both makes the key poll run once per frame no matter which loop
/// is pumping.
const CLOCK_UPDATE_CALL_OFFSETS: [u32; 2] = [0xb8ae2, 0xb8f7e];

const MAX_SCALE: u32 = 8;

/// The time multiplier the detour applies. Read by the assembly stub every frame.
static SCALE: AtomicU32 = AtomicU32::new(1);
static CLOCK_CALL_HOOKS: [AtomicPtr<CallRel32Hook>; 2] = [AtomicPtr::new(std::ptr::null_mut()), AtomicPtr::new(std::ptr::null_mut())];
static PLUS_WAS_DOWN: AtomicBool = AtomicBool::new(false);
static MINUS_WAS_DOWN: AtomicBool = AtomicBool::new(false);

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

    for (offset, slot) in CLOCK_UPDATE_CALL_OFFSETS.iter().zip(CLOCK_CALL_HOOKS.iter()) {
        match hook_call_rel32(*offset, clock_update_hook as usize as u32) {
            Ok(hook) => slot.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
            Err(_) => {
                error!("failed to hook the clock update call at module offset {offset:#x}");
                return 3;
            }
        }
    }

    info!("loaded: ALT+plus/minus scale time x1/x2/x4/x8");
    0
}

fn key_down(key: VIRTUAL_KEY) -> bool {
    (unsafe { GetKeyState(key.0 as i32) } as u16) & 0x8000 != 0
}

/// Edge detection: true once per press. The swap must happen unconditionally -
/// short-circuiting it made keys stick in a previous mod.
fn edge(down: bool, was_down: &AtomicBool) -> bool {
    let previously_down = was_down.swap(down, Ordering::Relaxed);
    down && !previously_down
}

/// Runs once per frame in every message loop: forward to the game's clock updater,
/// then poll ALT+plus / ALT+minus and step the scale through 1 / 2 / 4 / 8.
///
/// The edge state tracks the plus/minus keys regardless of ALT, so holding a key
/// and then pressing ALT does not fire - a press is only a press if ALT was already
/// down when the key went down.
unsafe extern "C" fn clock_update_hook() {
    let orig: extern "C" fn() = mem::transmute((*CLOCK_CALL_HOOKS[0].load(Ordering::SeqCst)).old_absolute);
    orig();

    let alt = key_down(VK_MENU);
    let up = edge(key_down(VK_OEM_PLUS), &PLUS_WAS_DOWN) && alt;
    let down = edge(key_down(VK_OEM_MINUS), &MINUS_WAS_DOWN) && alt;
    if up == down {
        return;
    }
    let scale = SCALE.load(Ordering::Relaxed);
    let new = if up { (scale * 2).min(MAX_SCALE) } else { (scale / 2).max(1) };
    if new != scale {
        SCALE.store(new, Ordering::Relaxed);
        notify(&format!("extra speed: x{new}"));
    }
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
