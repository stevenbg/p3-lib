//! Rescues a trade route that stalled on "crew number too low".
//!
//! A route ship that loses sailors to pirates finishes its trading at the next stop and
//! then cannot depart: the game clears its course (`0x00519C40`), deactivates the route,
//! and files the note rendered as `%s's trade route: crew number too low`. The player
//! then walks to the tavern, hires, and re-activates by hand.
//!
//! This module does that by hook: the note's creation site (`0x00518AB2`, see
//! [p3_api::letters]) is intercepted, and if the shortfall can be covered from the
//! tavern of the town the ship is docked in, the sailors are hired (the game's own hire
//! operation, opcode 0x04) and the route re-activated (opcode 0x68, the same operation
//! the route window's button sends) - and the note is never filed. If the tavern cannot
//! cover the shortfall, the original call runs and the message appears exactly as in
//! vanilla.
//!
//! **A convoy stalls through a different door.** Its lead ship is checked by two
//! predicates instead of the lone ship's inline crew compare - [ShipPtr::can_lead_convoy]
//! then [ShipPtr::can_sail] - and a failure of either files "%s's trade route is
//! interrupted" (subtype `0x22`) from two further sites. There is no shared point to
//! catch both kinds: the checks live in different routines and only the note creator
//! `0x00548CA0` is common, and hooking that would intercept every letter in the game. So
//! all three call sites are hooked instead.
//!
//! The convoy note is filed for any unmet requirement - a captainless lead ship, too few
//! guns, a hull under half, a frozen port - and only the crew is fixable by hiring. The
//! handler therefore asks the game itself: it re-runs both predicates against a copy of
//! the ship record carrying the crew a hire would produce ([ShipPtr::would_sail_with_crew])
//! and only hires when that copy would sail. Anything else falls through to vanilla.

use std::{
    mem,
    sync::atomic::{AtomicU32, Ordering},
};

use hooklet::windows::x86::hook_call_rel32;
use log::{error, info, warn};
use p3_api::{
    game_world::GAME_WORLD_PTR,
    letters::{
        ROUTE_NOTE_CREW_TOO_LOW_CALL_SITE_OFFSET, ROUTE_NOTE_INTERRUPTED_CONVOY_STOP_CALL_SITE_OFFSET,
        ROUTE_NOTE_INTERRUPTED_TICK_CALL_SITE_OFFSET, ROUTE_NOTE_KIND_CREW_TOO_LOW, ROUTE_NOTE_KIND_ROUTE_INTERRUPTED,
    },
    operation::Operation,
    operations::{execute_operation, OPERATIONS_PTR},
    ship::ShipPtr,
    ships::ShipsPtr,
};

/// Every site that files a note this module answers, with what to call it in the log.
const NOTE_SITES: [(u32, &str); 3] = [
    (ROUTE_NOTE_CREW_TOO_LOW_CALL_SITE_OFFSET, "the lone ship's crew-too-low note"),
    (ROUTE_NOTE_INTERRUPTED_CONVOY_STOP_CALL_SITE_OFFSET, "the convoy stop's interrupted note"),
    (ROUTE_NOTE_INTERRUPTED_TICK_CALL_SITE_OFFSET, "the convoy tick's interrupted note"),
];
/// The note creator every site calls (`0x00548CA0`), taken from the first hook and
/// checked against the others: one fallthrough target serves all three.
static NOTE_CREATOR: AtomicU32 = AtomicU32::new(0);

pub(crate) unsafe fn install() -> Result<(), &'static str> {
    for (site, what) in NOTE_SITES {
        let Ok(hook) = hook_call_rel32(site, route_note_hook as *const () as u32) else {
            return Err("failed to hook a route-note call site");
        };
        let creator = hook.old_absolute;
        // Leaked on purpose: the hooks live as long as the process, and dropping one
        // would restore the original call.
        let _ = Box::into_raw(Box::new(hook));
        match NOTE_CREATOR.compare_exchange(0, creator, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => info!("crew rescue: hooked {what}, creator {creator:#010x}"),
            Err(first) if first == creator => info!("crew rescue: hooked {what}"),
            Err(first) => {
                error!("crew rescue: {what} calls {creator:#010x}, not {first:#010x} - not the same creator");
                return Err("a route-note call site points somewhere unexpected");
            }
        }
    }
    Ok(())
}

/// Stands in for the note creator `0x00548CA0` at the three sites in [NOTE_SITES].
/// Everything but a successful rescue falls through to the original, so every other note
/// kind and every failed rescue behaves exactly like vanilla.
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
    // A convoy's note names its lead ship, which is the ship both convoy predicates are
    // evaluated on, so the rescue acts on the same ship either way.
    let rescued = match kind {
        ROUTE_NOTE_KIND_CREW_TOO_LOW => try_rescue(ship_index, false),
        ROUTE_NOTE_KIND_ROUTE_INTERRUPTED => try_rescue(ship_index, true),
        _ => false,
    };
    if rescued {
        // The caller ignores the return value (it just zeroes ship+0x138 after).
        return 0;
    }
    info!("route note: passing through to the game");
    let original: extern "thiscall" fn(u32, u32, u32, u32, u32, u32, u32) -> u32 =
        mem::transmute(NOTE_CREATOR.load(Ordering::SeqCst));
    original(this, merchant, zero1, zero2, town, kind, ship_index)
}

/// True when the shortfall was hired and the route re-activated. The hire and the
/// re-activation are executed directly (not enqueued): the deactivation this reacts to
/// already happened synchronously, and the note must be suppressed in the same breath.
unsafe fn try_rescue(ship_index: u32, as_convoy_leader: bool) -> bool {
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
    let crew = ship.get_crew();
    // A convoy's lead ship answers to 20, not to its type's minimum.
    let wanted = if as_convoy_leader {
        u16::from(ship.get_min_sailors()).max(ShipPtr::CONVOY_LEADER_MIN_CREW)
    } else {
        u16::from(ship.get_min_sailors())
    };
    let needed = wanted as i32 - crew as i32;
    if needed <= 0 {
        info!("crew rescue: crew {crew} already at/above {wanted} - the crew is not what stopped it");
        return false;
    }
    // For a convoy the note covers every unmet requirement, so hire only when the crew is
    // the last one: ask the game's own predicates what the ship would do with the hire.
    if as_convoy_leader && !ship.would_sail_with_crew(wanted, true) {
        info!(
            "crew rescue: ship {ship_index} would still not sail with {wanted} sailors (leader {}, sail {}) - something other than the crew is unmet",
            ship.can_lead_convoy(),
            ship.can_sail()
        );
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
