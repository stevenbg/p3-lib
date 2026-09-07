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

use hooklet::windows::x86::{hook_call_rel32, hook_function_pointer, CallRel32Hook, FunctionPointerHook};
use log::{error, info};
use p3_api::{
    battle::{BattlePtr, BattleShipPtr},
    operations::OPERATIONS_PTR,
    ships::ShipsPtr,
    ui::ui_notifications::UINotificationsPtr,
};

static MAIN_STEP_HOOK: AtomicPtr<FunctionPointerHook> = AtomicPtr::new(std::ptr::null_mut());
static TWIN_STEP_HOOK: AtomicPtr<FunctionPointerHook> = AtomicPtr::new(std::ptr::null_mut());
static OPERATION_HOOK: AtomicPtr<CallRel32Hook> = AtomicPtr::new(std::ptr::null_mut());

/// The queue drain's call to the operation switch, module-relative for `hook_call_rel32`.
const OPERATION_SWITCH_CALL_OFFSET: u32 = 0x0014_6934;

/// The side scale that makes the AI's own strength comparisons fail, so its ships choose to
/// flee. See [p3_api::battle::BattlePtr::get_side_scale].
const COWARD_SCALE: u8 = 0;

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
            error!(
                "battle-ship vtable slot 0 at {:#010x} reads {slot:#010x}, expected {expected:#010x} - not patching",
                0x0040_0000 + offset
            );
            return 1;
        }
    }

    match hook_function_pointer(
        BattleShipPtr::VTABLE_MAIN_OFFSET + BattleShipPtr::SLOT_STEP * 4,
        main_step_hook as *const () as usize as u32,
    ) {
        Ok(hook) => MAIN_STEP_HOOK.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
        Err(_) => {
            error!("failed to hook the battle-ship step (main vtable)");
            return 2;
        }
    }
    match hook_function_pointer(
        BattleShipPtr::VTABLE_TWIN_OFFSET + BattleShipPtr::SLOT_STEP * 4,
        twin_step_hook as *const () as usize as u32,
    ) {
        Ok(hook) => TWIN_STEP_HOOK.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
        Err(_) => {
            error!("failed to hook the battle-ship step (twin vtable)");
            return 3;
        }
    }

    match hook_call_rel32(OPERATION_SWITCH_CALL_OFFSET, operation_hook as *const () as u32) {
        Ok(hook) => OPERATION_HOOK.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
        Err(_) => {
            error!("failed to hook the operation switch call - battle orders will not be logged");
            return 4;
        }
    }

    info!("battle always-flee installed: the player's ships flee every battle the player has not entered");
    0
}

/// Slot 0 of `0x0067AAD0`: `thiscall(this, step_argument)`, `ret 4`.
unsafe extern "thiscall" fn main_step_hook(this: u32, step_argument: u32) {
    let original: extern "thiscall" fn(u32, u32) = mem::transmute((*MAIN_STEP_HOOK.load(Ordering::SeqCst)).old_absolute);
    step(this, step_argument, original);
}

/// Slot 0 of `0x0067B0D4`, the twin.
unsafe extern "thiscall" fn twin_step_hook(this: u32, step_argument: u32) {
    let original: extern "thiscall" fn(u32, u32) = mem::transmute((*TWIN_STEP_HOOK.load(Ordering::SeqCst)).old_absolute);
    step(this, step_argument, original);
}

/// One ship's step, with this side's courage taken away **only for the duration of this
/// ship's own evaluation** and put back straight afterwards. The scale lives on the battle
/// side, so leaving it at zero would make every ship of the side a coward, an ally's
/// included; the evaluation reads it inside this call and nowhere else, so bracketing the
/// call is what turns a side-wide setting into a per-ship one.
unsafe fn step(this: u32, step_argument: u32, original: extern "thiscall" fn(u32, u32)) {
    let ship_object = BattleShipPtr::new(this);
    let entry = ship_object.get_flags();
    let grace_entry = ship_object.get_grace_counter();

    let cowardly = should_flee(this);
    // Once per ship per battle, let the AI decide at its real courage so the log can say
    // whether this mod changed anything - see [crate::probe::wants_verdict_sample].
    let sample = cowardly.is_some() && crate::probe::wants_verdict_sample(this);
    let restore = cowardly.filter(|_| !sample).map(|battle| {
        let side = ship_object.get_side();
        let previous = battle.get_side_scale(side);
        battle.set_side_scale(side, COWARD_SCALE);
        (battle, side, previous)
    });

    original(this, step_argument);

    if let Some((battle, side, previous)) = restore {
        battle.set_side_scale(side, previous);
    }
    if sample {
        if let Some(battle) = cowardly {
            let name = ShipsPtr::new()
                .get_ship(ship_object.get_ship_index() as u16)
                .map(|s| s.get_name())
                .unwrap_or_else(|| "<unknown>".into());
            crate::probe::note_verdict(this, &name, &battle, battle.get_side_scale(ship_object.get_side()));
        }
    }

    crate::probe::note_step(this, entry, ship_object.get_flags(), grace_entry);
    after_step(this);
}

/// The battle to make this ship flee in, or `None` to leave it entirely alone: it must be
/// one of the player's ships, still in the fight, in a battle the player has **not**
/// entered and did **not** start. A town assault is the player's own attack, so it is left
/// to be fought.
unsafe fn should_flee(object: u32) -> Option<BattlePtr> {
    let ship_object = BattleShipPtr::new(object);
    if !ship_object.is_in_fight() {
        return None;
    }
    let ship = ShipsPtr::new().get_ship(ship_object.get_ship_index() as u16)?;
    let player = OPERATIONS_PTR.get_player_merchant_index();
    if player < 0 || ship.get_merchant_index() as i32 != player {
        return None;
    }
    let battle = ship_object.get_battle()?;
    if battle.is_player_battle() || battle.is_attacker(player as u16) {
        return None;
    }
    Some(battle)
}

/// The operation switch, at the queue drain's call to it (`0x00546934` -> `0x00535760`),
/// `thiscall(op)`. Only the battle orders `0x93`..`0x9B` are logged, with the addressed
/// battle's ships either side of the switch, so what the flee button does is a diff.
unsafe extern "thiscall" fn operation_hook(op: u32) {
    let original: extern "thiscall" fn(u32) = mem::transmute((*OPERATION_HOOK.load(Ordering::SeqCst)).old_absolute);
    let battle = crate::probe::note_operation_before(op);
    original(op);
    if let Some(battle) = battle {
        crate::probe::note_operation_after(&battle);
    }
}

/// After the game's own step: report once per battle what the mod is doing there. The
/// fleeing itself is the AI's own decision now, taken inside the step - see [step].
unsafe fn after_step(object: u32) {
    let ship_object = BattleShipPtr::new(object);
    crate::probe::observe(object);
    let Some(ship) = ShipsPtr::new().get_ship(ship_object.get_ship_index() as u16) else {
        return;
    };
    let Some(battle) = ship_object.get_battle() else { return };
    if should_flee(object).is_none() {
        return;
    }
    if ANNOUNCED_BATTLE.swap(battle.address, Ordering::SeqCst) != battle.address {
        let text = format!("{}: forced to flee", ship.get_name());
        info!(
            "{text} (battle slot {}, side {}, owners {}/{}, town {})",
            battle.get_slot(),
            ship_object.get_side(),
            battle.get_side_owner(0),
            battle.get_side_owner(1),
            battle.is_town_battle()
        );
        notify(&text);
    }
}

/// A ticker popup, in the game's codepage.
unsafe fn notify(text: &str) {
    let latin1: Vec<u8> = text.chars().map(|c| if (c as u32) <= 0xff { c as u32 as u8 } else { b'?' }).collect();
    UINotificationsPtr::new().post_event(&latin1);
}
