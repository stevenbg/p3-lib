//! Fixes the church window's **uninitialised mode**, at its source.
//!
//! The window's mode lives at `+0x1D30`, and its constructor (`0x005C88E0`) never writes
//! it - it initialises the neighbouring `+0x1D5F` two instructions before returning, and
//! misses this one. Only four sites in the whole executable write the field: the close
//! method (`0x005C94AC`, to `-1`), `set_mode` (`0x005C9DD1`), and two page setters
//! (`0x005CB08C`, `0x005CB7A3`). So a freshly constructed window carries whatever its heap
//! block came with.
//!
//! That single defect has two faces, and both were seen in play:
//!
//! - **The crash.** After a quit-to-menu the window is reconstructed from a *recycled*
//!   block carrying a stale mode `> 0`. The tick then decides between `switch_animation`
//!   and `load_next_frame` by animation id alone - "not the intro, so it must be my loop,
//!   just stream the next frame" - and with a freshly constructed player (current `0xFF`,
//!   frame array NULL) that writes through NULL at `0x0046B49D`.
//! - **A blank first page.** On a cold start the block reads `0`, so the window dispatches
//!   page 0 instead of the `-1` empty page it settles on after its first close. Harmless
//!   in vanilla, but it is the same uninitialised field, and it is what made the defect
//!   easy to see: `mod-church-details` draws on page `-1`, so its page was missing on the
//!   first open of a session and present on every one after. Reported 29 Aug 2026.
//!
//! **The fix is to initialise the field.** The constructor's single call site
//! (`0x00426BBD`, whose result is stored to the window global at `0x00426BD6`) is hooked,
//! and the mode set to `-1` - the same value the close method uses - on the object the
//! constructor returns. That removes the crash's precondition rather than catching its
//! consequence, and gives the window the empty page on a cold open as a side effect.
//!
//! The previous fix - three byte patches in the tick, plus guards on the animation
//! player's unguarded methods - is preserved whole in [crate::tick_patch] and is **not
//! installed**. If the crash ever returns, call `tick_patch::install()` from [start]: the
//! two are independent and can run together.
//!
//! Background: `.claude/notes/todo/church-window-anim-crash.md`.

use std::{
    mem,
    sync::atomic::{AtomicPtr, Ordering},
};

use hooklet::windows::x86::{hook_call_rel32, CallRel32Hook};
use log::{error, info};

/// The `call 0x005C88E0` at `0x00426BBD` - the church window constructor's only call site.
/// `thiscall(this) -> this`, no stack arguments, plain `ret`.
///
/// Module-relative for `hooklet`'s `hook_call_rel32`, which adds the module base itself.
const CONSTRUCTOR_CALL_OFFSET: u32 = 0x00026BBD;

/// The window's mode/page field, left uninitialised by the constructor.
const WINDOW_MODE_OFFSET: u32 = 0x1D30;

/// What the close method (`0x005C94AC`) writes, and so what the window's own code treats
/// as "no page selected".
const MODE_NONE: i32 = -1;

static CONSTRUCTOR_HOOK: AtomicPtr<CallRel32Hook> = AtomicPtr::new(std::ptr::null_mut());

#[no_mangle]
pub unsafe extern "C" fn start() -> u32 {
    let _ = log::set_logger(&win_dbg_logger::DEBUGGER_LOGGER);
    log::set_max_level(log::LevelFilter::Info);

    match hook_call_rel32(CONSTRUCTOR_CALL_OFFSET, church_window_ctor_hook as *const () as u32) {
        Ok(hook) => CONSTRUCTOR_HOOK.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
        Err(_) => {
            error!("failed to hook the church window constructor - mode stays uninitialised");
            return 1;
        }
    }

    info!("church mode fix installed: the window's mode is initialised to {MODE_NONE}");
    0
}

/// `0x005C88E0(this) -> this` - the church window constructor. Runs it, then writes the
/// mode the constructor forgot.
///
/// Writing after the original rather than before is deliberate: the constructor zeroes and
/// populates the object, so anything written first would be at risk of being overwritten.
/// It returns `this` in eax, which is what the call site stores to the window global.
#[no_mangle]
unsafe extern "thiscall" fn church_window_ctor_hook(this: u32) -> u32 {
    let original: extern "thiscall" fn(u32) -> u32 =
        mem::transmute((*CONSTRUCTOR_HOOK.load(Ordering::SeqCst)).old_absolute);
    let window = original(this);
    if window != 0 {
        *((window + WINDOW_MODE_OFFSET) as *mut i32) = MODE_NONE;
    }
    window
}
