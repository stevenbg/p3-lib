//! Small quality-of-life tweaks to the game's own UI.
//!
//! Current tweaks:
//! - Letter popups name their town: the incoming-letter notifications on the top
//!   right ("Personal letter: Patrol") get the letter's town appended ("Personal
//!   letter: Patrol - Stockholm"), so mission letters say at a glance where to
//!   send the ship.

use log::{error, info, warn};
use std::mem;
use std::sync::atomic::{AtomicPtr, AtomicU32, Ordering};

use hooklet::windows::x86::{hook_call_rel32, CallRel32Hook};

/// The game's MFC-style string objects: a single pointer to character data whose
/// header lives in the 12 bytes before it. [0x6C7CD0] holds the shared empty-string
/// header; a fresh object starts as that sentinel + 0xC so the constructor's
/// release-old-data path is a no-op.
const STRING_EMPTY_HEADER_PTR: *const u32 = 0x006c7cd0 as _;
/// thiscall(this, *const u8) -> this: construct/assign from a C string.
const STRING_CTOR_FROM_CSTR: u32 = 0x0064f390;
/// thiscall(this): release the string data.
const STRING_DTOR: u32 = 0x0064f253;

/// Incoming-letter popups ("Personal letter: Patrol") get the letter's town appended
/// ("Personal letter: Patrol - Stockholm"). Scripted letters carry a garbage town
/// byte (the patrol-letter bug), but their formatted text names the destination: the
/// LAST town name occurring in the text ("...get your ship to <town> as soon as
/// possible"). Simple letters carry a valid town byte directly. Two call hooks: the
/// mailbox insert's announcer call (0x4D66E0 -> announcer 0x4D7B10) stashes the
/// message being announced; the announcer's right-ticker enqueue call (0x4D7D12 ->
/// 0x42BB20) then rebuilds the popup string with the town appended.
const ANNOUNCER_CALL_OFFSET: u32 = 0x000d66e0;
const RIGHT_TICKER_CALL_OFFSET: u32 = 0x000d7d12;
/// The message currently being announced (set around the announcer call), 0 = none.
static ANNOUNCED_MESSAGE: AtomicU32 = AtomicU32::new(0);
static ANNOUNCER_HOOK: AtomicPtr<CallRel32Hook> = AtomicPtr::new(std::ptr::null_mut());
static RIGHT_TICKER_HOOK: AtomicPtr<CallRel32Hook> = AtomicPtr::new(std::ptr::null_mut());

/// Sets the PEB BeingDebugged flag so IsDebuggerPresent() returns true, unlocking the
/// gated win_dbg_logger used by every mod. No real debugger is attached, so log output
/// still reaches DebugView via OutputDebugString. Lives here (rather than in a
/// specific automation mod) so it is easy to find and remove later.
#[cfg(target_arch = "x86")]
unsafe fn fake_being_debugged() {
    let peb: *mut u8;
    std::arch::asm!("mov {}, fs:[0x30]", out(reg) peb);
    // PEB + 0x02 = BeingDebugged (u8).
    *peb.add(2) = 1;
}

#[cfg(not(target_arch = "x86"))]
unsafe fn fake_being_debugged() {}

#[no_mangle]
pub unsafe extern "C" fn start() -> u32 {
    let _ = log::set_logger(&win_dbg_logger::DEBUGGER_LOGGER);
    log::set_max_level(log::LevelFilter::Trace);

    fake_being_debugged();

    match hook_call_rel32(ANNOUNCER_CALL_OFFSET, announcer_hook as usize as u32) {
        Ok(hook) => ANNOUNCER_HOOK.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
        Err(_) => {
            error!("failed to hook the letter announcer call");
            return 1;
        }
    }
    match hook_call_rel32(RIGHT_TICKER_CALL_OFFSET, right_ticker_hook as usize as u32) {
        Ok(hook) => RIGHT_TICKER_HOOK.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
        Err(_) => {
            error!("failed to hook the letter ticker call");
            return 2;
        }
    }
    info!("loaded: letter popups name their town");
    0
}

unsafe extern "thiscall" fn announcer_hook(this: u32, merchant: u32, message: u32) {
    ANNOUNCED_MESSAGE.store(message, Ordering::SeqCst);
    let original: extern "thiscall" fn(u32, u32, u32) = mem::transmute((*ANNOUNCER_HOOK.load(Ordering::SeqCst)).old_absolute);
    original(this, merchant, message);
    ANNOUNCED_MESSAGE.store(0, Ordering::SeqCst);
}

/// The right-ticker enqueue takes an MFC string BY VALUE (one data-pointer slot, the
/// callee releases it). To augment, release the incoming string ourselves and hand
/// the original a fresh one.
unsafe extern "thiscall" fn right_ticker_hook(manager: u32, text: u32) {
    let original: extern "thiscall" fn(u32, u32) = mem::transmute((*RIGHT_TICKER_HOOK.load(Ordering::SeqCst)).old_absolute);
    let message = ANNOUNCED_MESSAGE.load(Ordering::SeqCst);
    let town = if message != 0 { letter_town_name(message) } else { None };
    let Some(town) = town else {
        original(manager, text);
        return;
    };
    let mut augmented: Vec<u8> = Vec::new();
    let mut p = text;
    loop {
        let b = *(p as *const u8);
        if b == 0 || augmented.len() >= 96 {
            break;
        }
        augmented.push(b);
        p += 1;
    }
    augmented.extend_from_slice(b" - ");
    augmented.extend_from_slice(&town);
    augmented.push(0);

    let ctor: extern "thiscall" fn(*mut u32, *const u8) -> *mut u32 = mem::transmute(STRING_CTOR_FROM_CSTR);
    let dtor: extern "thiscall" fn(*mut u32) = mem::transmute(STRING_DTOR);
    let mut incoming = text;
    dtor(&mut incoming);
    let mut replacement: u32 = *STRING_EMPTY_HEADER_PTR + 0xc;
    ctor(&mut replacement, augmented.as_ptr());
    original(manager, replacement);
}

/// A town's name bytes when long enough to be worth matching or appending (the
/// letter text is in the game's codepage, so all work happens on raw bytes).
fn town_name_bytes(town_index: u8) -> Option<Vec<u8>> {
    p3_api::town::get_town_name_bytes(town_index).filter(|name| name.len() >= 3)
}

/// The town a letter is about, as raw name bytes. Scripted letters: the last town
/// name occurring in the letter text (bounded by the length the creation handler
/// stores at descriptor+0). Simple letters: the town byte.
unsafe fn letter_town_name(message: u32) -> Option<Vec<u8>> {
    let msg_type = *((message + 4) as *const u8);
    if !(0x3c..=0x40).contains(&msg_type) && msg_type != 0x71 {
        let town_byte = *((message + 5) as *const u8);
        if town_byte >= p3_api::town::TOWN_NAME_SLOTS {
            return None;
        }
        return town_name_bytes(town_byte);
    }
    let text = *((message + 0xc) as *const u32);
    let descriptor = *((message + 8) as *const u32);
    if text == 0 || descriptor == 0 {
        return None;
    }
    // descriptor+0 holds the text LENGTH once the letter is complete (observed 0xA6
    // for a patrol letter); tolerate an end pointer too, in case other paths leave
    // the raw write position.
    let raw = *(descriptor as *const u32);
    let length = if raw > 0 && raw <= 0x1000 {
        raw
    } else if raw > text && raw - text <= 0x1000 {
        raw - text
    } else {
        warn!(
            "letter town scan: descriptor length looks wrong (type {msg_type:#04x}, text {text:#010x}, desc {descriptor:#010x}, raw {raw:#010x})"
        );
        return None;
    };
    let hay = core::slice::from_raw_parts(text as *const u8, length as usize);
    let mut best: Option<(usize, Vec<u8>)> = None;
    let mut seen: Vec<Vec<u8>> = Vec::new();
    for town_index in 0..p3_api::town::TOWN_NAME_SLOTS {
        // Unfilled bank slots repeat the first town's name - scan each name once.
        let Some(name) = town_name_bytes(town_index) else { continue };
        if seen.contains(&name) {
            continue;
        }
        seen.push(name.clone());
        let Some(pos) = hay.windows(name.len()).rposition(|w| w == &name[..]) else {
            continue;
        };
        if best.as_ref().map(|(p, _)| pos > *p).unwrap_or(true) {
            best = Some((pos, name));
        }
    }
    best.map(|(_, name)| name)
}
