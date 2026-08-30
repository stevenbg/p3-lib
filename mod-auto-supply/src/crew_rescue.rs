//! Rescues a trade route that stalled on "crew number too low".
//!
//! A route ship that loses sailors to pirates finishes its trading at the next stop and
//! then cannot depart: the game clears its course (`0x00519C40`), deactivates the route,
//! and files the note rendered as `%s's trade route: crew number too low`. The player
//! then walks to the tavern, hires, and re-activates by hand.
//!
//! This module does that by hook: the note's single creation site (`0x00518AB2`, see
//! [p3_api::letters]) is intercepted, and if the shortfall can be covered from the
//! tavern of the town the ship is docked in, the sailors are hired (the game's own hire
//! operation, opcode 0x04) and the route re-activated (opcode 0x68, the same operation
//! the route window's button sends) - and the note is never filed. If the tavern cannot
//! cover the shortfall, the original call runs and the message appears exactly as in
//! vanilla.

use std::{
    mem,
    sync::atomic::{AtomicPtr, Ordering},
};

use hooklet::windows::x86::{hook_call_rel32, CallRel32Hook};
use log::{info, warn};
use p3_api::{
    game_world::GAME_WORLD_PTR,
    letters::{ROUTE_NOTE_CREW_TOO_LOW_CALL_SITE_OFFSET, ROUTE_NOTE_KIND_CREW_TOO_LOW},
    operation::Operation,
    operations::{execute_operation, OPERATIONS_PTR},
    ships::ShipsPtr,
};

static NOTE_HOOK: AtomicPtr<CallRel32Hook> = AtomicPtr::new(std::ptr::null_mut());

pub(crate) unsafe fn install() -> Result<(), &'static str> {
    match hook_call_rel32(ROUTE_NOTE_CREW_TOO_LOW_CALL_SITE_OFFSET, route_note_hook as *const () as u32) {
        Ok(hook) => {
            NOTE_HOOK.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst);
            info!("crew rescue: hooked the route-note call site");
            Ok(())
        }
        Err(_) => Err("failed to hook the route-note call site"),
    }
}

/// Stands in for the note creator `0x00548CA0` at its kind-5 call site. Everything but
/// a successful rescue falls through to the original, so every other note kind (this
/// site only ever sends 5, but the guard is free) and every failed rescue behaves
/// exactly like vanilla.
unsafe extern "thiscall" fn route_note_hook(
    this: u32,
    merchant: u32,
    zero1: u32,
    zero2: u32,
    town: u32,
    kind: u32,
    ship_index: u32,
) -> u32 {
    info!("route note fired: kind={kind} ship={ship_index} town={town} merchant={merchant}");
    // The creator's ship argument is a 16-bit index pushed as a dword with stale upper
    // bits (measured: 0x006D007D for ship 0x7D) - the callee only reads the word. Mask
    // before using it anywhere a dword is expected, or the hire handler's bounds check
    // rejects it.
    let ship_index = ship_index & 0xFFFF;
    if kind == ROUTE_NOTE_KIND_CREW_TOO_LOW && try_rescue(ship_index) {
        // The caller ignores the return value (it just zeroes ship+0x138 after).
        return 0;
    }
    info!("route note: passing through to the game");
    let original: extern "thiscall" fn(u32, u32, u32, u32, u32, u32, u32) -> u32 =
        mem::transmute((*NOTE_HOOK.load(Ordering::SeqCst)).old_absolute);
    original(this, merchant, zero1, zero2, town, kind, ship_index)
}

/// True when the shortfall was hired and the route re-activated. The hire and the
/// re-activation are executed directly (not enqueued): the deactivation this reacts to
/// already happened synchronously, and the note must be suppressed in the same breath.
unsafe fn try_rescue(ship_index: u32) -> bool {
    let Some(ship) = ShipsPtr::new().get_ship(ship_index as u16) else {
        warn!("crew rescue: ship index {ship_index} does not resolve");
        return false;
    };
    let player = OPERATIONS_PTR.get_player_merchant_index();
    info!(
        "crew rescue: ship {ship_index} '{}' at {:#010x}: owner {} (player {player}), type {:#04x}, crew {}, min {}, town {:?}, status {:#x}, +0x138 {:#x}",
        ship.get_name(),
        ship.address,
        ship.get_merchant_index(),
        *((ship.address + 0x0e) as *const u8),
        ship.get_crew(),
        ship.get_min_sailors(),
        ship.get_last_town_index(),
        ship.get_status(),
        *((ship.address + 0x138) as *const u16),
    );
    if player < 0 || ship.get_merchant_index() as i32 != player {
        info!("crew rescue: not the player's ship (owner {}, player {player})", ship.get_merchant_index());
        return false;
    }
    if !ship.is_in_port() {
        info!("crew rescue: ship not in port (status {:#x})", ship.get_status());
        return false;
    }
    let Some(town_index) = ship.get_last_town_index() else {
        info!("crew rescue: ship has no town");
        return false;
    };
    let crew = ship.get_crew() as i32;
    let needed = ship.get_min_sailors() as i32 - crew;
    if needed <= 0 {
        info!("crew rescue: crew {crew} already at/above the minimum - not touching it");
        return false;
    }
    let available = GAME_WORLD_PTR.get_merchant(player as u16).get_available_sailors(town_index) as i32;
    if available < needed {
        info!("crew rescue: ship {ship_index} needs {needed}, tavern has {available} - letting the message through");
        return false;
    }
    execute_operation(&Operation::HireSailors {
        ship_index,
        count: needed as u32,
    });
    execute_operation(&Operation::SetTradeRouteActive { ship_index, active: true });
    crate::ffi::notify(&format!("{}: hired {needed} sailors, route resumed", ship.get_name()));
    true
}
