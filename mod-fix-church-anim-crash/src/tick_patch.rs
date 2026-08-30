//! **DORMANT.** This was the shipped fix until 29 Aug 2026, when the root cause was
//! found and fixed at its source in `ffi.rs`; [install] is no longer called.
//!
//! It is kept, whole and compiling, because it is the fallback: if the church window ever
//! crashes on its animation again, calling [install] from `start()` restores exactly the
//! behaviour that had soaked without a report. Deleting it would mean re-deriving three
//! byte patches and their justification from scratch.
//!
//! Everything below describes that fix, not the current one.
//!
//! Fixes the church window crashing after a save load (NULL frame array).
//!
//! Building-interior windows share one animation player (global `[0x006CC7F4]`): a
//! frame-slot array at `+0x4`, the current animation id at `+0x38` (`0xFF` = none).
//! Only `switch_animation` (`0x0046B710`) allocates the array; `load_next_frame`
//! (`0x0046B110`) stores straight into it. The church window's tick decides between
//! the two by *animation id comparison only*: "the current animation is not the
//! one-shot intro, so it must already be my loop - just stream its next frame".
//!
//! That assumption breaks after a save load. The window's mode (`+0x1D30`) is never
//! initialized by its constructor, and the recreated window usually inherits the
//! previous session's mode through the recycled heap block. With a stale mode > 0
//! and a freshly constructed player (current = `0xFF`, array = NULL), the first tick
//! after the church opens calls `load_next_frame` and writes through NULL at
//! `0x0046B49D` - the crash. The same branch also mis-fires when the player holds
//! another building's animation: frames then stream into an array sized and cursored
//! for the wrong animation, a silent heap overflow.
//!
//! The fix is three bytes in the tick, turning "not the intro" into "exactly my
//! loop animation" and routing every other state through `switch_animation`, which
//! allocates, loads frame 0 and positions the sprite - the same pattern the window's
//! own set_mode epilogue (`0x005CA166`) already uses:
//!
//! - `0x005C95E2`: retarget the intro branch's mismatch fall-through (`jne` rel8
//!   `0x59` -> `0x47`) into the loop-branch dispatcher at `0x005C962A` instead of
//!   the raw `load_next_frame` call.
//! - `0x005C9630`: compare the current animation against the loop id in `ebp`
//!   instead of the intro id in `esi` (`cmp eax,esi` -> `cmp eax,ebp`).
//! - `0x005C9631`: invert the jump (`jne` -> `je`): equal -> stream next frame,
//!   anything else -> `switch_animation`.
//!
//! Evidence that the fix engages: the `switch_animation` call at `0x005C9635` is
//! hooked, and any invocation whose prior state would have crashed (NULL array) or
//! corrupted (foreign animation current) is appended to `_church_anim_fix.log`.
//! As belt and braces the player's three unguarded methods - `load_next_frame`,
//! `tick` (`0x0046B530`) and `draw` (`0x0046B6A0`) - are detoured to bail out and
//! log when the array is NULL. The only other user of the player, the tavern
//! window, was audited and is safe (every stream call sits behind a
//! current-animation check, and its teardown resets the id to `0xFF` at
//! `0x005CD503`), so these guards are pure insurance against a missed path. The
//! file logging is temporary, to collect evidence during normal play, and comes
//! out once the fix has soaked.
//!
//! Background: `.claude/notes/todo/church-window-anim-crash.md`.
#![allow(dead_code)]

use std::{
    arch::global_asm,
    ffi::c_void,
    io::Write,
    mem,
    sync::atomic::{AtomicPtr, AtomicU32, Ordering},
};

use hooklet::windows::x86::{deploy_rel32_raw, hook_call_rel32, CallRel32Hook, X86Rel32Type};
use log::{error, info, warn};
use windows::Win32::System::Memory::{VirtualProtect, PAGE_EXECUTE_READWRITE, PAGE_PROTECTION_FLAGS};

/// The three byte patches in the church window's tick, with the bytes they replace.
/// Each entry is (address, expected original, replacement).
const TICK_PATCHES: [(u32, u8, u8); 3] = [
    // jne rel8: retarget the intro-branch fall-through from the load_next_frame call
    // (0x005C963C) to the loop-branch dispatcher (0x005C962A).
    (0x005C95E2, 0x59, 0x47),
    // cmp eax,esi -> cmp eax,ebp: test against the loop animation, not the intro.
    (0x005C9630, 0xC6, 0xC5),
    // jne -> je: equal streams the next frame, everything else switches properly.
    (0x005C9631, 0x75, 0x74),
];

/// `call 0x0046B710` at `0x005C9635` - the tick's switch_animation call, which the
/// patched branches now route every unhealthy state through (module-relative, as
/// `hook_call_rel32` takes it).
const SWITCH_CALL_OFFSET: u32 = 0x001C9635;

/// The shared animation player global and its fields.
const PLAYER_PTR_ADDRESS: *const u32 = 0x006CC7F4 as _;
const PLAYER_FRAMES_OFFSET: u32 = 0x4;
const PLAYER_FRAME_INDEX_OFFSET: u32 = 0x1F;
const PLAYER_LOADED_COUNT_OFFSET: u32 = 0x20;
const PLAYER_CURRENT_ANIM_OFFSET: u32 = 0x38;

/// Church window fields, logged for context: the mode whose stale value arms the
/// bug, the intro flag, the church stage that picks the animation pair, the town.
const WINDOW_MODE_OFFSET: u32 = 0x1D30;
const WINDOW_TOWN_OFFSET: u32 = 0x1D38;
const WINDOW_STAGE_OFFSET: u32 = 0x1D5E;
const WINDOW_INTRO_FLAG_OFFSET: u32 = 0x1D5F;

/// `load_next_frame`: `0x0046B110(this, anim)`, `ret 0x4`. The detour overwrites
/// `push -1` and most of `push 0x0065F20A`; the stub re-executes both.
const LOAD_PATCH_ADDRESS: u32 = 0x0046B110;
static LOAD_CONTINUATION: u32 = 0x0046B117;
/// `tick`: `0x0046B530(this, anim, a2, a3)`, `ret 0xC`. Stolen instructions:
/// `sub esp,0x1C` and `xor eax,eax`.
const TICK_PATCH_ADDRESS: u32 = 0x0046B530;
static TICK_CONTINUATION: u32 = 0x0046B535;
/// `draw`: `0x0046B6A0(this)`, plain `ret`. Stolen: `mov eax,[0x006CC3E8]`.
const DRAW_PATCH_ADDRESS: u32 = 0x0046B6A0;
static DRAW_CONTINUATION: u32 = 0x0046B6A5;

/// Next to `_crash_report.txt`: the modloader's working directory is the game
/// folder. Never truncated, so hits accumulate across sessions.
const LOG_FILE: &str = "_church_anim_fix.log";

const SITE_NAMES: [&str; 3] = ["load", "tick", "draw"];
/// A NULL array makes tick/draw fire every frame until something switches, so the
/// file log mutes each site after this many lines (the guard keeps rejecting).
const SITE_LOG_CAP: u32 = 25;

static SWITCH_HOOK: AtomicPtr<CallRel32Hook> = AtomicPtr::new(std::ptr::null_mut());

/// Installs the tick fix. **Not called** - see the module header.
pub(crate) unsafe fn install() -> u32 {
    // The fix itself: three bytes in the church tick. Verify before writing so a
    // different exe version fails loudly instead of corrupting code.
    for (address, expected, _) in TICK_PATCHES {
        let found = *(address as *const u8);
        if found != expected {
            error!("unexpected byte at {address:#010x}: found {found:#04x}, expected {expected:#04x} - not patching");
            return 1;
        }
    }
    let patch_base = TICK_PATCHES[0].0;
    let patch_len = (TICK_PATCHES[2].0 - patch_base + 1) as usize;
    let mut old_flags = PAGE_PROTECTION_FLAGS(0);
    if !VirtualProtect(patch_base as _, patch_len, PAGE_EXECUTE_READWRITE, &mut old_flags).as_bool() {
        error!("VirtualProtect PAGE_EXECUTE_READWRITE failed");
        return 2;
    }
    for (address, _, replacement) in TICK_PATCHES {
        *(address as *mut u8) = replacement;
    }
    if !VirtualProtect(patch_base as _, patch_len, old_flags, &mut old_flags).as_bool() {
        error!("VirtualProtect restore failed");
        return 3;
    }

    // The evidence hook on the switch call the patched branches route through.
    match hook_call_rel32(SWITCH_CALL_OFFSET, switch_animation_hook as *const () as u32) {
        Ok(hook) => SWITCH_HOOK.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
        Err(_) => {
            error!("failed to hook the tick's switch_animation call");
            return 4;
        }
    }

    // Belt and braces: never let the player methods touch a NULL frame array.
    if deploy_rel32_raw(LOAD_PATCH_ADDRESS as _, (&load_detour) as *const _ as _, X86Rel32Type::Jump).is_err() {
        error!("failed to detour load_next_frame at {LOAD_PATCH_ADDRESS:#010x}");
        return 5;
    }
    if deploy_rel32_raw(TICK_PATCH_ADDRESS as _, (&tick_detour) as *const _ as _, X86Rel32Type::Jump).is_err() {
        error!("failed to detour tick at {TICK_PATCH_ADDRESS:#010x}");
        return 6;
    }
    if deploy_rel32_raw(DRAW_PATCH_ADDRESS as _, (&draw_detour) as *const _ as _, X86Rel32Type::Jump).is_err() {
        error!("failed to detour draw at {DRAW_PATCH_ADDRESS:#010x}");
        return 7;
    }

    info!("church anim fix installed: tick patched, switch hooked, player methods guarded");
    0
}

/// `0x0046B710(this, anim, window)` - switch_animation, thiscall with two stack
/// arguments. Called by the patched tick for every state that is not "my loop
/// animation is current". A state the vanilla branches handled identically (the
/// intro just finished) passes silently; anything else is the bug firing, and is
/// logged with everything needed to reconstruct how the window got there.
#[no_mangle]
unsafe extern "thiscall" fn switch_animation_hook(this: u32, anim: u32, window: u32) {
    let original: extern "thiscall" fn(u32, u32, u32) =
        mem::transmute((*SWITCH_HOOK.load(Ordering::SeqCst)).old_absolute);

    let frames = *((this + PLAYER_FRAMES_OFFSET) as *const u32);
    let current = *((this + PLAYER_CURRENT_ANIM_OFFSET) as *const u8) as u32;
    // The pairs are (loop, intro) = (6, 7) and (8, 9): the intro handing over to
    // the loop is the one transition the vanilla comparison also got right.
    let vanilla_transition = frames != 0 && current == anim + 1;
    if !vanilla_transition {
        let loaded = *((this + PLAYER_LOADED_COUNT_OFFSET) as *const u8);
        let frame = *((this + PLAYER_FRAME_INDEX_OFFSET) as *const u8);
        let mode = *((window + WINDOW_MODE_OFFSET) as *const i32);
        let town = *((window + WINDOW_TOWN_OFFSET) as *const i32);
        let stage = *((window + WINDOW_STAGE_OFFSET) as *const u8);
        let intro_flag = *((window + WINDOW_INTRO_FLAG_OFFSET) as *const u8);
        let count = HEAL_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
        let effect = if frames == 0 { "was a NULL-array crash" } else { "was a wrong-array stream" };
        let line = format!(
            "healed switch #{count}: anim={anim} current={current:#04x} frames={frames:#010x} \
             loaded={loaded} frame={frame} mode={mode} intro_flag={intro_flag} stage={stage} \
             town={town} ({effect})",
        );
        warn!("{line}");
        append_log_line(&line);
    }

    original(this, anim, window);
}

static HEAL_COUNT: AtomicU32 = AtomicU32::new(0);

/// 1 = the frame array exists, carry on into the original method; 0 = reject, the
/// stub returns without dereferencing NULL. Called from the three asm stubs with
/// the player's `this` and the animation argument (-1 for draw, which takes none).
///
/// With the tick patch in place nothing here should ever fire: the church is fixed
/// and the tavern - the player's only other user - guards all its own calls. A
/// `load`/`tick`/`draw` rejection therefore means the audit missed a path, and the
/// log says which method and with which animation ids.
#[no_mangle]
unsafe extern "C" fn anim_player_guard(site: u32, this: u32, anim: i32) -> u32 {
    if this != 0 && *((this + PLAYER_FRAMES_OFFSET) as *const u32) != 0 {
        return 1;
    }

    let site_index = site.min(2) as usize;
    let count = SITE_COUNTS[site_index].fetch_add(1, Ordering::Relaxed) + 1;
    if count <= SITE_LOG_CAP {
        let (current, loaded) = if this != 0 {
            (
                *((this + PLAYER_CURRENT_ANIM_OFFSET) as *const u8) as i32,
                *((this + PLAYER_LOADED_COUNT_OFFSET) as *const u8) as i32,
            )
        } else {
            (-1, -1)
        };
        let line = format!(
            "rejected {} #{count}: this={this:#010x} anim={anim} current={current:#04x} loaded={loaded} (frame array is NULL)",
            SITE_NAMES[site_index],
        );
        warn!("{line}");
        append_log_line(&line);
        if count == SITE_LOG_CAP {
            append_log_line(&format!("muting {} after {SITE_LOG_CAP} lines, still guarding", SITE_NAMES[site_index]));
        }
    }
    0
}

/// Rejections so far this session, per method, so the cap works per site.
static SITE_COUNTS: [AtomicU32; 3] = [AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0)];

/// Temporary evidence file - a hit is a crash or a heap corruption that did not
/// happen. Failures to write are swallowed: the guard must never be the thing
/// that breaks the frame.
fn append_log_line(line: &str) {
    let unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(LOG_FILE) {
        let _ = writeln!(file, "[unix {unix}] {line}");
    }
}

extern "C" {
    static load_detour: c_void;
    static tick_detour: c_void;
    static draw_detour: c_void;
}

// All three stubs run at the very top of their method, before its prologue: ecx is
// the player and only ecx must survive the guard call (each original moves it into
// esi right after the stolen instructions; eax and edx are dead on both paths).
// The guard is cdecl, so arguments go on in reverse: anim, this, site.

// load_next_frame: stack is [ret][anim]; a rejected call returns `ret 0x4` like the
// original, with eax cleared since the store it feeds never happened.
global_asm!("
.global {load_detour}
{load_detour}:
push ecx
mov eax, dword ptr [esp + 8]
push eax
push ecx
push 0
call {guard}
add esp, 12
pop ecx
test eax, eax
je 2f
# the two instructions the jmp overwrote, then back into the original
push -1
push 0x0065F20A
jmp [{load_continuation}]
2:
xor eax, eax
ret 4
",
load_detour = sym load_detour,
guard = sym anim_player_guard,
load_continuation = sym LOAD_CONTINUATION);

// tick: stack is [ret][anim][a2][a3]; a rejected call returns `ret 0xC`.
global_asm!("
.global {tick_detour}
{tick_detour}:
push ecx
mov eax, dword ptr [esp + 8]
push eax
push ecx
push 1
call {guard}
add esp, 12
pop ecx
test eax, eax
je 2f
# the two instructions the jmp overwrote, then back into the original
sub esp, 0x1C
xor eax, eax
jmp [{tick_continuation}]
2:
ret 12
",
tick_detour = sym tick_detour,
guard = sym anim_player_guard,
tick_continuation = sym TICK_CONTINUATION);

// draw: no stack arguments, plain `ret`.
global_asm!("
.global {draw_detour}
{draw_detour}:
push ecx
push -1
push ecx
push 2
call {guard}
add esp, 12
pop ecx
test eax, eax
je 2f
# the instruction the jmp overwrote, then back into the original
mov eax, dword ptr [0x006CC3E8]
jmp [{draw_continuation}]
2:
ret
",
draw_detour = sym draw_detour,
guard = sym anim_player_guard,
draw_continuation = sym DRAW_CONTINUATION);
