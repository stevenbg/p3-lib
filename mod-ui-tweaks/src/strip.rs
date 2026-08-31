//! CTRL+X: strip the selected ship bare into the town's office and send it to the yard.
//! Registration lives in [crate::ffi::start].

use num_traits::FromPrimitive;
use p3_api::{
    data::enums::WareId,
    game_world::GAME_WORLD_PTR,
    operation::{Operation, CUTLASS_WEAPON_TYPE},
    operations::{execute_operation, OPERATIONS_PTR},
    ships::ShipsPtr,
    town::get_town_name,
    ui::ui_ship_panel::UIShipPanelPtr,
};

/// The artillery types a ship can carry, [p3_api::data::enums::ShipWeaponId] `0..=5`.
const ARTILLERY_TYPES: std::ops::Range<u8> = 0..6;

/// CTRL+X: everything that has to happen to a freshly captured ship - its cargo and
/// weapons into the office, its crew paid off, and the hull booked in for repair.
///
/// The four steps in order: cargo ware by ware (opcode 0x08, `0x00538100`), then each
/// artillery type and the cutlasses (opcode 0x09, `0x00538210`), then the whole crew
/// (opcode 0x05), and last the repair (opcode 0x03, `0x0052ACD0`). Repair goes last
/// because it puts the ship into status `4` and every transfer above needs `< 4`.
///
/// The operations are executed rather than enqueued: a full ship is over thirty of them,
/// which is most of the pending queue's 52 slots, and executing keeps the order exact.
///
/// Refused for a convoy's lead ship - the weapons operation rejects one anyway unless the
/// convoy carries flag `0x2`.
///
/// Declines (0) when no ship is selected or the ship is not the player's, so the key stays
/// visible to the game; consumes it once it acts or has something to say.
pub(crate) unsafe extern "C" fn strip_ship_hotkey(_vk: u32, _mods: u32) -> u32 {
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
    let (Some(town_index), true) = (ship.get_last_town_index(), ship.is_docked()) else {
        crate::ffi::notify(&format!("{name}: not docked in a harbour"));
        return 1;
    };
    let town = get_town_name(town_index).unwrap_or_else(|| "<unknown>".into());
    if let Some(convoy) = ships.get_convoy(ship.get_convoy_id()) {
        if convoy.get_lead_ship_index() == ship_index {
            crate::ffi::notify(&format!("{name}: a convoy's lead ship cannot be stripped"));
            return 1;
        }
    }
    if GAME_WORLD_PTR.get_office_in_of(town_index, player as u16).is_none() {
        crate::ffi::notify(&format!("{name}: no trading office in {town}"));
        return 1;
    }

    let wares = ship.get_wares();
    let mut ware_count = 0;
    for ware_index in 0u16..24 {
        let amount = wares[ware_index as usize];
        if amount <= 0 {
            continue;
        }
        execute_operation(&Operation::ShipMoveWares {
            amount,
            ship_index: ship_index as u16,
            ware_id: WareId::from_u16(ware_index).unwrap(),
            merchant_index: player as u16,
            to_ship: false,
        });
        ware_count += 1;
    }

    let slots = ship.get_artillery_slots();
    let mut guns = 0;
    for weapon_type in ARTILLERY_TYPES {
        // One gun per type byte: a large weapon's second slot holds the 6 marker, not
        // the type, so counting type bytes counts weapons rather than slots.
        let count = slots.iter().filter(|&&slot| slot == weapon_type).count() as i32;
        if count == 0 {
            continue;
        }
        execute_operation(&Operation::ShipMoveWeapons {
            weapon_type: weapon_type as u32,
            town_index: town_index as u32,
            ship_index: ship_index as u32,
            amount: count,
            to_ship: false,
        });
        guns += count;
    }

    let cutlasses = ship.get_cutlasses() as i32;
    if cutlasses > 0 {
        execute_operation(&Operation::ShipMoveWeapons {
            weapon_type: CUTLASS_WEAPON_TYPE,
            town_index: town_index as u32,
            ship_index: ship_index as u32,
            amount: cutlasses,
            to_ship: false,
        });
    }

    let crew = ship.get_crew() as i32;
    if crew > 0 {
        execute_operation(&Operation::DismissSailors {
            ship_index: ship_index as u32,
            count: crew as u32,
        });
    }

    execute_operation(&Operation::RepairShip {
        ship_index: ship_index as u32,
    });

    crate::ffi::notify(&format!(
        "{name} stripped in {town}: {ware_count} wares, {guns} guns, {cutlasses} cutlasses, {crew} sailors - repair ordered"
    ));
    1
}
