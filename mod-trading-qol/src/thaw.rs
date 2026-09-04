//! Restarts automatic trade when a frozen port thaws.
//!
//! The unfreeze-port scheduled task (opcode 0x35, handler `0x004E94A4`) clears the
//! town's frozen flag; this module hooks its single call site in the task dispatcher
//! and, after the original has run, re-activates this mod's route ships that serve the
//! town. Which ships those are is read off the generated ship names (the route keys
//! stamp `-` plus one three-letter town code per stop), so no route walking is needed.

use std::{
    mem,
    sync::atomic::{AtomicPtr, Ordering},
};

use hooklet::windows::x86::{hook_call_rel32, CallRel32Hook};
use log::{debug, error, info};
use p3_api::{
    game_world::GAME_WORLD_PTR,
    operation::Operation,
    operations::OPERATIONS_PTR,
    scheduled_tasks::{scheduled_task::SCHEDULED_TASK_OPCODE_UNFREEZE_PORT, SCHEDULED_TASKS_PTR},
    town::get_town_name,
};

/// Module-relative offset of the only call to the unfreeze-port scheduled task
/// (`0x004E94A4`, opcode `0x35`), inside the task dispatcher.
const UNFREEZE_PORT_CALL_OFFSET: u32 = 0x000D89C8;

static UNFREEZE_HOOK_PTR: AtomicPtr<CallRel32Hook> = AtomicPtr::new(std::ptr::null_mut());

pub(crate) unsafe fn install() -> Result<(), &'static str> {
    match hook_call_rel32(UNFREEZE_PORT_CALL_OFFSET, unfreeze_port_hook as *const () as u32) {
        Ok(hook) => {
            UNFREEZE_HOOK_PTR.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst);
            Ok(())
        }
        Err(_) => Err("failed to hook the unfreeze-port task"),
    }
}

/// A port thawing: `thiscall` on the scheduled-tasks singleton, no arguments. The task
/// being executed is the one at `tasks+0x8`, which is how the handler itself finds the
/// town index - so we read it the same way rather than guessing.
///
/// The original runs first, so the frozen flag is already clear when we resume ships.
unsafe extern "thiscall" fn unfreeze_port_hook(tasks: u32) {
    let original: extern "thiscall" fn(u32) =
        mem::transmute((*UNFREEZE_HOOK_PTR.load(Ordering::SeqCst)).old_absolute);

    let task = SCHEDULED_TASKS_PTR.get_scheduled_task(SCHEDULED_TASKS_PTR.get_earliest_task_index());
    // Only trust the town index if the task really is the one we expect: the hook fires
    // from a single call site, but the index is read out of shared mutable state.
    let town_index = (task.get_opcode() == SCHEDULED_TASK_OPCODE_UNFREEZE_PORT)
        .then(|| task.get_data_dword(0));

    original(tasks);

    match town_index {
        Some(index) if index < GAME_WORLD_PTR.get_towns_count() as u32 => {
            resume_autotrade_after_thaw(index as u8)
        }
        Some(index) => error!("thaw: task town index {index} out of range, no ships resumed"),
        None => error!("thaw: unfreeze hook fired on opcode {:#x}, no ships resumed", task.get_opcode()),
    }
}

/// Restart automatic trade on this mod's route ships that serve a town whose port has
/// just thawed.
///
/// A frozen port turns arriving ships away, and an auto-trade ship that was routed
/// through it can end up stopped - annoying to notice and to restart by hand, since
/// nothing in the game tells you which ships were affected. The route keys already stamp a
/// generated ship's route into its name (`route_ship_name` (ffi.rs): [crate::routes::ROUTE_NAME_PREFIX] then one
/// [crate::routes::town_code] per route town), so the name is a reliable, cheap statement of "this ship
/// serves that town" - no route walking needed.
///
/// Only ever *sets* the flag, and only on the player's own ships whose generated name
/// names this town. A ship already trading is left alone, so the hook is a no-op in the
/// common case and can never stop a ship.
unsafe fn resume_autotrade_after_thaw(town_index: u8) {
    let Some(code) = crate::routes::town_code(town_index) else {
        error!("thaw: town {town_index} has no name, no ships resumed");
        return;
    };
    let town = get_town_name(town_index).unwrap_or_else(|| format!("town {town_index}"));
    let player = *(0x006DFC14 as *const u32) as u8;
    let ships = p3_api::ships::ShipsPtr::new();

    let mut resumed = Vec::new();
    let mut already = 0;
    for ship_index in 0..ships.get_ships_size() {
        let Some(ship) = ships.get_ship(ship_index) else { continue };
        if ship.get_merchant_index() != player {
            continue;
        }
        let name = ship.get_name();
        if !name.starts_with(crate::routes::ROUTE_NAME_PREFIX) || !name.contains(&code) {
            continue;
        }
        if ship.is_trade_route_active() {
            already += 1;
            continue;
        }
        OPERATIONS_PTR.enqueue_operation(Operation::SetTradeRouteActive {
            ship_index: ship_index as u32,
            active: true,
        });
        resumed.push(name);
    }

    if resumed.is_empty() {
        debug!("thaw in {town} ({code}): {already} route ships already trading, none to resume");
        return;
    }
    info!(
        "thaw in {town} ({code}): resumed automatic trade on [{}] ({already} already trading)",
        resumed.join(", ")
    );
    crate::ffi::notify(&format!("{town} ice-free: restarted {} ship(s)", resumed.len()));
}
