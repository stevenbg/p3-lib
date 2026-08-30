//! The crew keys: hire from the tavern, or dismiss into the town, without opening a
//! single window. Registration lives in [crate::ffi::start]; everything else about the
//! feature is here.

use p3_api::{
    game_world::GAME_WORLD_PTR,
    hotkeys::{MOD_ALT, MOD_CTRL},
    operation::Operation,
    operations::OPERATIONS_PTR,
    ships::ShipsPtr,
    town::get_town_name,
    ui::ui_ship_panel::UIShipPanelPtr,
};

/// CTRL+S / ALT+S / CTRL+ALT+S: crew the selected ship from (or into) the town it is
/// docked in.
///
/// - **CTRL+S fills to the maximum** the game itself would hire: the hire cap at
///   `0x005184F0` - the type's full crew (table `0x673664`) plus a cargo-derived term,
///   bounded by the room left.
/// - **ALT+S hires the bare sailing minimum**: up to the type's minimum (the game's
///   table at `0x673660`, via [p3_api::ship::ShipPtr::get_min_sailors]) - the cheap
///   option for a trader that only needs to move.
/// - **CTRL+ALT+S dismisses the whole crew** into the town (opcode 0x05, `0x00537DD0`:
///   they rejoin it as beggars and citizens; crew and morale go to zero).
///
/// The hires go through the real hire operation (opcode 0x04, `0x00537C20`), which
/// re-clamps every request and does the bookkeeping (sailor pool, town beggars, convoy
/// refresh).
///
/// Declines (0) when no ship is selected or the ship is not the player's, so the key
/// stays visible to the game; consumes it once it acts or has something to say.
pub(crate) unsafe extern "C" fn hire_sailors_hotkey(_vk: u32, mods: u32) -> u32 {
    let Some(ship_index) = UIShipPanelPtr::new().get_selected_ship_index() else {
        return 0;
    };
    let Some(ship) = ShipsPtr::new().get_ship(ship_index) else {
        return 0;
    };
    let player = OPERATIONS_PTR.get_player_merchant_index();
    if player < 0 || ship.get_merchant_index() as i32 != player {
        return 0;
    }
    let name = ship.get_name();
    // Docked, not merely in town: a ship still entering the port (status 3) can already
    // reach the tavern by the game's own loose rule, but these keys wait for the quay.
    let (Some(town_index), true) = (ship.get_last_town_index(), ship.is_docked()) else {
        crate::ffi::notify(&format!("{name}: not docked in a harbour"));
        return 1;
    };
    let crew = ship.get_crew() as i32;
    if mods & MOD_CTRL != 0 && mods & MOD_ALT != 0 {
        if crew == 0 {
            crate::ffi::notify(&format!("{name}: no sailors aboard"));
            return 1;
        }
        OPERATIONS_PTR.enqueue_operation(Operation::DismissSailors {
            ship_index: ship_index as u32,
            count: crew as u32,
        });
        crate::ffi::notify(&format!("{name}: dismissing all {crew} sailors"));
        return 1;
    }
    let to_minimum = mods & MOD_ALT != 0;
    let wanted = if to_minimum {
        let min_sailors = ship.get_min_sailors() as i32;
        if crew >= min_sailors {
            crate::ffi::notify(&format!("{name}: already sailable ({crew} sailors, needs {min_sailors})"));
            return 1;
        }
        min_sailors - crew
    } else {
        let wanted = ship.get_free_sailor_berths();
        if wanted <= 0 {
            crate::ffi::notify(&format!("{name}: crew is already full ({crew} sailors)"));
            return 1;
        }
        wanted
    };
    let available = GAME_WORLD_PTR.get_merchant(player as u16).get_available_sailors(town_index) as i32;
    if available == 0 {
        let town = get_town_name(town_index).unwrap_or_else(|| format!("town {town_index}"));
        crate::ffi::notify(&format!("{name}: no sailors in the tavern of {town}"));
        return 1;
    }
    let count = wanted.min(available);
    OPERATIONS_PTR.enqueue_operation(Operation::HireSailors {
        ship_index: ship_index as u32,
        count: count as u32,
    });
    let goal = if to_minimum { "the minimum" } else { "full" };
    let note = if count < wanted { " - tavern ran out, still short" } else { "" };
    crate::ffi::notify(&format!("{name}: hiring {count} sailors toward {goal} ({crew} -> {}){note}", crew + count));
    1
}
