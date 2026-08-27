//! Fixes a hard freeze in the captain-retirement scheduled task (opcode `0x27`,
//! handler `0x004DDC00`).
//!
//! The ten-day captain scan flags a captain who has passed 50 years and schedules this
//! task with the ship he was on and his auto-trader index. Because he may have moved in
//! the meantime, the handler's first job is to re-find him - and that search contains a
//! two-instruction infinite loop:
//!
//! ```text
//! 4ddcab: mov cx,[edx+esi*1+0x42]   ; the examined ship's captain
//! 4ddcb0: cmp ecx,ebp               ; the captain we want
//! 4ddcb2: 75 fc  jne 0x4ddcb0       ; displacement -4, back to the compare
//! ```
//!
//! Nothing between those two instructions writes `ecx` or `ebp`, so a mismatch spins
//! forever. Two further defects in the same block show it was never exercised: the
//! "advance to the next ship in the chain" step (`ship+0x4`) is missing entirely, and
//! `0x004DDC9F` loads the ships array into `esi`, which is also the merchant loop's own
//! down-counter.
//!
//! **Reproduced 28 Aug 2026** by scheduling the task by hand with a mismatched pair: the
//! game froze with one core at 100% (5.06s of CPU in 5.0s of wall clock), `Responding`
//! false, and no crash report - nothing faults, so there is nothing for
//! mod-crash-reporter to catch. See `.claude/notes/todo/captain-retire-hang.md`.
//!
//! In normal play the mismatch needs the captain to leave the recorded ship inside the
//! scheduling window: one tick for a human owner's captain (operation `0x13`,
//! `0x00538C8F`), but **three hours** for an AI owner's (`0x004DCFFF`) - and AI ships are
//! sunk and sold constantly, so the exposure is mostly not the player's own fleet.
//!
//! ## The fix
//!
//! A hook on the handler's only call site (`0x004D88EF`) that resolves the pair before
//! delegating, so the broken search is never entered:
//!
//! - **pair still matches** - the overwhelmingly common case, every normal retirement:
//!   call the original untouched. It sees a match, skips the search, and does its work.
//! - **captain found on another ship**: rewrite the task's ship index and call the
//!   original, which then also sees a match. This is what the broken code was trying to
//!   do - retire him from the ship he is actually on.
//! - **captain on no ship at all** (dismissed, or his ship sank): return `0` without
//!   calling the original. That is exactly what the game itself does at its own bail-out
//!   `0x004DDE2D` (`xor eax,eax; ret`) when its search finds nothing, and the dispatcher
//!   reads `0` as "task done, free it" (`0x004D8A12` compares against `ebp`, zeroed at
//!   `0x004D85EB`). There is no ship to take him off, so there is nothing to do.
//!
//! Deliberately NOT used: the handler's own `due += 0x20` self-postpone (its convoy
//! branch at `0x004DDCFC` does this). A captain who has permanently left his ship would
//! be rescheduled every three hours forever - benign churn instead of a hang, but a worse
//! trade than dropping the task.
//!
//! The mod logs only when it actually intervenes, which in normal play is never.
use std::mem;
use std::sync::atomic::{AtomicPtr, AtomicU32, Ordering};

use hooklet::windows::x86::{hook_call_rel32, CallRel32Hook};
use log::{error, info, warn};
use p3_api::{
    data::p3_ptr::P3Pointer,
    game_world::GAME_WORLD_PTR,
    scheduled_tasks::{
        scheduled_task::{ScheduledTaskPtr, SCHEDULED_TASK_SIZE},
        ScheduledTasksPtr,
    },
    ships::ShipsPtr,
};

/// Module-relative offset of the only call to the retirement handler `0x004DDC00`,
/// inside the scheduled-task dispatcher's opcode switch.
const HANDLER_CALL_OFFSET: u32 = 0x000D88EF;
/// The opcode this task carries, checked before touching anything.
const RETIRE_TASK_OPCODE: u16 = 0x27;
const LOG_FILE: &str = "_captain_retire_fix.log";

static HANDLER_HOOK: AtomicPtr<CallRel32Hook> = AtomicPtr::new(std::ptr::null_mut());
static REPAIRED: AtomicU32 = AtomicU32::new(0);
static DROPPED: AtomicU32 = AtomicU32::new(0);

#[no_mangle]
pub unsafe extern "C" fn start() -> u32 {
    let _ = log::set_logger(&win_dbg_logger::DEBUGGER_LOGGER);
    log::set_max_level(log::LevelFilter::Info);

    match hook_call_rel32(HANDLER_CALL_OFFSET, retire_handler_hook as usize as u32) {
        Ok(hook) => HANDLER_HOOK.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
        Err(_) => {
            error!("failed to hook the captain-retirement handler call - the freeze is NOT fixed");
            return 1;
        }
    }
    info!("captain-retirement hang fixed: the handler's broken ship search is bypassed");
    0
}

fn log_line(line: &str) {
    warn!("{line}");
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(LOG_FILE) {
        use std::io::Write;
        let _ = writeln!(file, "{line}");
    }
}

/// `thiscall` on the scheduled-tasks singleton, no arguments, returning the
/// keep-or-drop flag. The task being executed is the one at `tasks+0x8`, which is how
/// the handler itself finds its record.
#[no_mangle]
unsafe extern "thiscall" fn retire_handler_hook(tasks: u32) -> u32 {
    let original: extern "thiscall" fn(u32) -> u32 =
        mem::transmute((*HANDLER_HOOK.load(Ordering::SeqCst)).old_absolute);

    let Some((task, task_address)) = current_task(tasks) else {
        return original(tasks);
    };
    // Defensive: only this opcode has the layout below. Anything else is not ours.
    if task.get_opcode() != RETIRE_TASK_OPCODE {
        return original(tasks);
    }
    let recorded_ship = task.get_data_dword(0);
    let captain = task.get_data_dword(4);

    if pair_matches(recorded_ship, captain) {
        // The normal path: the handler skips its search entirely.
        return original(tasks);
    }

    match find_captains_ship(captain) {
        Some((ship_index, name, merchant)) => {
            // The handler re-reads the data union, so rewriting the ship index here is
            // all it takes for it to see a matching pair and carry on correctly.
            *((task_address + 0x8) as *mut u32) = ship_index as u32;
            let count = REPAIRED.fetch_add(1, Ordering::Relaxed) + 1;
            log_line(&format!(
                "repair #{count}: captain {captain} was recorded on ship {recorded_ship} but is on ship {ship_index} {name:?} (merchant {merchant}); retiring him from there instead of hanging"
            ));
            original(tasks)
        }
        None => {
            let count = DROPPED.fetch_add(1, Ordering::Relaxed) + 1;
            log_line(&format!(
                "drop #{count}: captain {captain} was recorded on ship {recorded_ship} and is on no ship at all; dropping the task, as the game's own bail-out does"
            ));
            // The dispatcher frees the task on 0, exactly as after 0x004DDE2D.
            0
        }
    }
}

/// The record the dispatcher is currently executing, and its address.
unsafe fn current_task(tasks: u32) -> Option<(ScheduledTaskPtr, u32)> {
    let tasks_ptr = ScheduledTasksPtr { address: tasks };
    let index = tasks_ptr.get_earliest_task_index();
    if index >= tasks_ptr.get_tasks_size() {
        return None;
    }
    let base: u32 = tasks_ptr.get(0x0);
    if base == 0 {
        return None;
    }
    let address = base + index as u32 * SCHEDULED_TASK_SIZE;
    Some((ScheduledTaskPtr::new(address), address))
}

/// The handler's own entry test (`0x004DDC54`..`0x004DDC60`): the recorded ship index
/// must be in range and that ship must still carry the wanted captain.
unsafe fn pair_matches(ship_index: u32, captain: u32) -> bool {
    let ships = ShipsPtr::new();
    if ship_index >= ships.get_ships_size() as u32 {
        return false;
    }
    ships
        .get_ship(ship_index as u16)
        .is_some_and(|ship| ship.get_captain_index() as u32 == captain)
}

/// The search the handler meant to do. A linear pass over the ships array rather than
/// the merchant chains: it makes no assumption about chain consistency and cannot loop,
/// and it runs once per retirement - a rare event - over a few thousand entries.
///
/// A ship owned by a live merchant wins over an ownerless one, which is the case the
/// merchant-driven original would have found; an ownerless hull holding the captain is
/// still accepted rather than dropping the task, and is named in the log.
unsafe fn find_captains_ship(captain: u32) -> Option<(u16, String, u8)> {
    let ships = ShipsPtr::new();
    let merchants = GAME_WORLD_PTR.get_merchants_count();
    let mut fallback = None;
    for index in 0..ships.get_ships_size() {
        let Some(ship) = ships.get_ship(index) else { continue };
        if ship.get_captain_index() as u32 != captain {
            continue;
        }
        let merchant = ship.get_merchant_index();
        if (merchant as u16) < merchants {
            return Some((index, ship.get_name(), merchant));
        }
        if fallback.is_none() {
            fallback = Some((index, ship.get_name(), merchant));
        }
    }
    fallback
}
