//! The player's ships always try to flee a sea battle the player has not entered.
//!
//! A convoy's flagship has to carry guns, and an armed ship is exactly what the battle AI
//! is willing to fight with: its fight-or-flee decision (`0x00622804` in the per-ship step)
//! is skipped altogether while the ship is undamaged and nobody on its side has panicked,
//! and once it runs, an **unarmed** ship gets a free "flee because I cannot fight" branch
//! (`ship+0x120 == 0` and outnumbered) that an armed one does not. So the same auto-trader
//! that used to run from every pirate stands and fights the moment it is given a gun.
//!
//! Sending the flee operation (`0x9B`) once is useless: the AI re-decides the flag on its
//! next step. This mod hooks the per-ship AI step itself - slot 0 of both battle-ship
//! vtables ([BattleShipPtr::VTABLE_MAIN_OFFSET], [BattleShipPtr::VTABLE_TWIN_OFFSET]) -
//! runs the game's step, and then, for a ship of the player's, does what the flee button
//! does ([BattleShipPtr::order_flee]) and sets the side's panic latch, every step. The AI's
//! decision can never win, no code inside the 12 KB step is patched, and it works for either
//! side and either twin.
//!
//! **The battle the player has entered is left alone.** Forcing the flag there would cancel
//! the player's own steering the way the move order cancels a flee. That battle is the one
//! whose slot is in `[0x006E59CC]` ([p3_api::battle::BattlePtr::is_player_battle]), which the enter/decline
//! operation toggles; every other battle - the auto-resolved ones - gets the override.

use std::{
    mem,
    sync::atomic::{AtomicPtr, AtomicU32, Ordering},
};

use hooklet::windows::x86::{hook_function_pointer, FunctionPointerHook};
use log::{debug, error, info};
use p3_api::{
    battle::BattleShipPtr,
    operations::OPERATIONS_PTR,
    ships::ShipsPtr,
    ui::ui_notifications::UINotificationsPtr,
};

static MAIN_STEP_HOOK: AtomicPtr<FunctionPointerHook> = AtomicPtr::new(std::ptr::null_mut());
static TWIN_STEP_HOOK: AtomicPtr<FunctionPointerHook> = AtomicPtr::new(std::ptr::null_mut());

/// The battle the last ticker message was about, so each battle is announced once.
static ANNOUNCED_BATTLE: AtomicU32 = AtomicU32::new(0);

#[no_mangle]
pub unsafe extern "C" fn start() -> u32 {
    let _ = log::set_logger(&win_dbg_logger::DEBUGGER_LOGGER);
    log::set_max_level(log::LevelFilter::Info);

    // Refuse to hook a vtable whose slot 0 is not the step this mod knows: a different
    // build would put the override behind some other method.
    for (offset, expected) in [
        (BattleShipPtr::VTABLE_MAIN_OFFSET, BattleShipPtr::STEP_MAIN_ADDRESS),
        (BattleShipPtr::VTABLE_TWIN_OFFSET, BattleShipPtr::STEP_TWIN_ADDRESS),
    ] {
        let slot = *((0x0040_0000 + offset + BattleShipPtr::SLOT_STEP * 4) as *const u32);
        if slot != expected {
            error!("battle-ship vtable slot 0 at {:#010x} reads {slot:#010x}, expected {expected:#010x} - not patching", 0x0040_0000 + offset);
            return 1;
        }
    }

    match hook_function_pointer(BattleShipPtr::VTABLE_MAIN_OFFSET + BattleShipPtr::SLOT_STEP * 4, main_step_hook as *const () as usize as u32) {
        Ok(hook) => MAIN_STEP_HOOK.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
        Err(_) => {
            error!("failed to hook the battle-ship step (main vtable)");
            return 2;
        }
    }
    match hook_function_pointer(BattleShipPtr::VTABLE_TWIN_OFFSET + BattleShipPtr::SLOT_STEP * 4, twin_step_hook as *const () as usize as u32) {
        Ok(hook) => TWIN_STEP_HOOK.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
        Err(_) => {
            error!("failed to hook the battle-ship step (twin vtable)");
            return 3;
        }
    }

    info!("battle always-flee installed: the player's ships flee every battle the player has not entered");
    0
}

/// Slot 0 of `0x0067AAD0`: `thiscall(this, step_argument)`, `ret 4`.
unsafe extern "thiscall" fn main_step_hook(this: u32, step_argument: u32) {
    let original: extern "thiscall" fn(u32, u32) = mem::transmute((*MAIN_STEP_HOOK.load(Ordering::SeqCst)).old_absolute);
    original(this, step_argument);
    after_step(this);
}

/// Slot 0 of `0x0067B0D4`, the twin.
unsafe extern "thiscall" fn twin_step_hook(this: u32, step_argument: u32) {
    let original: extern "thiscall" fn(u32, u32) = mem::transmute((*TWIN_STEP_HOOK.load(Ordering::SeqCst)).old_absolute);
    original(this, step_argument);
    after_step(this);
}

/// After the game's own step: if this is a player's ship still in a battle the player has
/// not entered, re-issue the flee order and keep the side's panic latch set.
unsafe fn after_step(object: u32) {
    let ship_object = BattleShipPtr::new(object);
    if !ship_object.is_in_fight() {
        return;
    }
    let Some(ship) = ShipsPtr::new().get_ship(ship_object.get_ship_index() as u16) else {
        return;
    };
    let player = OPERATIONS_PTR.get_player_merchant_index();
    if player < 0 || ship.get_merchant_index() as i32 != player {
        return;
    }
    let Some(battle) = ship_object.get_battle() else {
        return;
    };
    if battle.is_player_battle() {
        if ANNOUNCED_BATTLE.swap(battle.address, Ordering::SeqCst) != battle.address {
            debug!("battle slot {} is the player's own - {} steers itself", battle.get_slot(), ship.get_name());
        }
        return;
    }

    let was_fleeing = ship_object.get_flags() & BattleShipPtr::FLAG_FLEEING != 0;
    ship_object.order_flee();
    battle.set_panic_latch(true);

    if ANNOUNCED_BATTLE.swap(battle.address, Ordering::SeqCst) != battle.address {
        let text = format!("{}: forced to flee", ship.get_name());
        info!("{text} (battle slot {}, marker +0x5C = {}, side {})", battle.get_slot(), battle.get_marker(), ship_object.get_side());
        notify(&text);
    } else if !was_fleeing {
        debug!("{}: the AI dropped the flee order, re-asserted", ship.get_name());
    }
}

/// A ticker popup, in the game's codepage.
unsafe fn notify(text: &str) {
    let latin1: Vec<u8> = text.chars().map(|c| if (c as u32) <= 0xff { c as u32 as u8 } else { b'?' }).collect();
    UINotificationsPtr::new().post_event(&latin1);
}
