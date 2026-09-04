//! CTRL+N: advance a route ship to the next stop. Registration lives in
//! [crate::ffi::start].

use p3_api::{
    data::route_stop::RouteStopPtr, operations::OPERATIONS_PTR, ship::ShipPtr, ships::ShipsPtr, town::get_town_name,
    ui::ui_ship_panel::UIShipPanelPtr,
};

/// CTRL+N: bump the selected own ship's **current route stop** (`ship+0x132`) to the
/// next stop in its route chain - the same field, and the same next-index the game reads
/// off the stop record, that the route executor advances when a ship finishes a stop
/// (`0x00503449`). Effectively "skip the stop it is heading for".
///
/// **The route lives on the convoy's lead ship**, so for a convoy this advances the
/// lead's stop, the ship the panel may not have named (it reports the clicked member).
///
/// Declines (0) when no ship is selected or it is not the player's, so the key stays
/// visible to the game; consumes it once it acts or has something to say.
pub(crate) unsafe extern "C" fn skip_route_stop_hotkey(_vk: u32, _mods: u32) -> u32 {
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

    // The route-holder: a convoy keeps its stops on the lead ship, the selected ship
    // otherwise.
    let route_ship: ShipPtr = match ships.get_convoy(ship.get_convoy_id()) {
        Some(convoy) => ships.get_ship(convoy.get_lead_ship_index()).unwrap_or(ship),
        None => ship,
    };

    let Some(current) = RouteStopPtr::from_pool_index(route_ship.get_route_stop_index()) else {
        crate::ffi::notify(&format!("{name}: no trade route"));
        return 1;
    };
    let next_index = current.get_next_index();
    let Some(next) = RouteStopPtr::from_pool_index(next_index) else {
        crate::ffi::notify(&format!("{name}: route stop chain is broken"));
        return 1;
    };

    route_ship.set_route_stop_index(next_index);

    let from = get_town_name(current.get_town_index()).unwrap_or_else(|| format!("town {}", current.get_town_index()));
    let to = get_town_name(next.get_town_index()).unwrap_or_else(|| format!("town {}", next.get_town_index()));
    crate::ffi::notify(&format!("{name}: skipped {from} -> next stop {to}"));
    1
}
