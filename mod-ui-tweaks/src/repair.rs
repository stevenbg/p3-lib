//! CTRL+R: book the selected ship into the town's shipyard. Registration lives in
//! [crate::ffi::start].

use p3_api::{
    operation::Operation, operations::OPERATIONS_PTR, ship::ShipPtr, ships::ShipsPtr, town::get_town_name,
    ui::ui_ship_panel::UIShipPanelPtr,
};

/// CTRL+R: order the repair the shipyard window's button orders - operation `0x03`,
/// whose handler `0x0052ACD0` deducts the cost, books it to the merchant and puts the
/// ship into status `4`, from where the next ships tick moves it into the yard's repair
/// chain (see `.claude/notes/done/ship-repair.md`).
///
/// **A convoy repairs as a whole, and the order goes to its lead ship.** The handler
/// sums the cost over every member of the convoy the ordered ship belongs to, so an
/// order on a member would be charged convoy-wide anyway; the game's own convoy route
/// stop resolves `convoy+0x10` - the lead ship - and orders on that
/// (`0x00503266`/`0x0050336B`), so this does the same. The panel reports the clicked
/// member rather than the lead, which is what makes the hop necessary.
///
/// Nothing is pre-checked beyond ownership and being docked: the handler answers an
/// undamaged hull or a short purse with the game's own letter, which is more informative
/// than a guess here would be.
///
/// Declines (0) when no ship is selected or the ship is not the player's, so the key
/// stays visible to the game; consumes it once it acts or has something to say.
pub(crate) unsafe extern "C" fn repair_ship_hotkey(_vk: u32, _mods: u32) -> u32 {
    let Some(ship_index) = UIShipPanelPtr::new().get_selected_ship_index() else {
        return 0;
    };
    let ships = ShipsPtr::new();
    let Some(ship) = ships.get_ship(ship_index) else {
        return 0;
    };
    let player = OPERATIONS_PTR.get_player_merchant_index();
    if player < 0 || ship.get_merchant_index() as i32 != player {
        return 0;
    }
    let name = ship.get_name();

    // The ship the order names, and what to call it: the convoy's lead ship where there
    // is one, the selected ship otherwise.
    let (target_index, target, whole_convoy) = match ships.get_convoy(ship.get_convoy_id()) {
        Some(convoy) => {
            let lead_index = convoy.get_lead_ship_index();
            match ships.get_ship(lead_index) {
                Some(lead) => (lead_index, lead, true),
                None => (ship_index, ship, false),
            }
        }
        None => (ship_index, ship, false),
    };
    // Docked means **lying in port**, status `0`, and not merely "in the port family":
    // the shipyard's own window lists nothing else, so a ship still entering the port or
    // already casting off is not a candidate here either - even though the operation
    // itself would accept it (its only test is `status < 4`).
    let (Some(town_index), true) = (target.get_last_town_index(), target.is_lying_in_port()) else {
        let state = target.get_in_port_state_name().unwrap_or("at sea");
        crate::ffi::notify(&format!("{name}: not docked - {state}"));
        return 1;
    };
    let town = get_town_name(town_index).unwrap_or_else(|| format!("town {town_index}"));

    OPERATIONS_PTR.enqueue_operation(Operation::RepairShip {
        ship_index: target_index as u32,
    });
    let hull = hull_percent(&target);
    if whole_convoy {
        crate::ffi::notify(&format!(
            "{name}: repair ordered in {town} for the whole convoy - lead ship {} at {hull}%",
            target.get_name()
        ));
    } else {
        crate::ffi::notify(&format!("{name}: repair ordered in {town} - hull at {hull}%"));
    }
    1
}

/// The ship's hull as a percentage of its maximum, the figure the shipyard window shows.
fn hull_percent(ship: &ShipPtr) -> u32 {
    let max = ship.get_max_health();
    if max == 0 {
        return 0;
    }
    (100 * ship.get_current_health().min(max)) / max
}
