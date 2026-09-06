//! Lets the player's captains train trade and combat up to their real ceilings, and
//! stops new auto-traders being rolled above them.
//!
//! **The gate bug.** The ten-day captain scan (`0x004DCEA0`) decides whether to grow a
//! captain by comparing all three of his skills against a single threshold `T`, and `T`
//! is the *navigation* ceiling of his slot - `SKILL_CAPS[index & 3]`, read from the gate
//! table at `0x00672824`. Trade and combat are never measured against their own
//! ceilings, so they freeze in `[T, T+49]` even when their real caps are 100 points
//! higher, and once all three sit at or above `T` the ship is skipped for good. The AI
//! branch has no gate at all, so this holds back **only the player's captains**, and it
//! bites on half of them (`T` is 250 for two slots in four, 200 for one, 150 for one).
//!
//! **The over-roll behind it.** The record initializer `0x004FDF50` rolls each skill
//! roughly uniformly over 0..255 under a *total* budget of 600 points, and never
//! consults the per-slot caps - so records are routinely born above their own ceilings
//! (46 of ~68 in a fresh campaign2 start). The gate bug hides that today: a frozen
//! record is skipped, so nothing ever clamps it.
//!
//! **Patch 1** raises the gate table to `250 250 250 250`. `T` stops binding, and the
//! *ceilings* keep being enforced by the gain handler's own table at `0x00673B34`, which
//! this mod does not touch. A refused roll and a clamped-to-cap gain have the same
//! outcome, so growth is identical to a correct per-skill gate; the only cost is a burnt
//! queue slot where vanilla skipped the ship. It cannot affect AI captains, because the
//! gate table is read only from the human branch (`0x004DD0EC`, `0x004DD0F7`).
//!
//! **Patch 2** clamps every newly rolled record to its slot's caps, hooking the
//! initializer's single call site at `0x00509897`, which hands us both the record and
//! its array index. Captains and pirates alike: the caps are the same table the game
//! already enforces on both, so this only moves enforcement from "first gain event" to
//! "at birth". Without it, patch 1 would expose every over-cap record to the handler's
//! clamp and *cost* skill, which is the opposite of the point.
//!
//! Records already over cap inside a save or campaign scenario were rolled before this
//! mod existed; the game's own clamp prunes them the first time a gain reaches them.
//! That is deliberate - it is what vanilla already does to any player captain who is not
//! fully frozen.
use std::{
    mem,
    sync::atomic::{AtomicPtr, Ordering},
};

use hooklet::windows::x86::{hook_call_rel32, CallRel32Hook};
use log::{error, info};
use p3_api::{
    auto_trader::{skill_caps, AutoTraderPtr, SKILL_CAPS, SKILL_GATE_TABLE_ADDRESS},
    memory::write_readonly,
};

/// What the gate table must contain before we touch it, and what we put there. Verifying
/// first means a different game build fails loudly instead of corrupting `.rdata`.
const GATE_EXPECTED: [u8; 4] = SKILL_CAPS;
const GATE_PATCHED: [u8; 4] = [250; 4];

/// `call 0x004FDF50` at `0x00509897`, the record initializer's only call site, inside the
/// only allocator (`0x005097C0`). Module-relative, as `hook_call_rel32` takes it.
const INITIALIZER_CALL_OFFSET: u32 = 0x0010_9897;

static INITIALIZER_HOOK: AtomicPtr<CallRel32Hook> = AtomicPtr::new(std::ptr::null_mut());

#[no_mangle]
pub unsafe extern "C" fn start() -> u32 {
    let _ = log::set_logger(&win_dbg_logger::DEBUGGER_LOGGER);
    log::set_max_level(log::LevelFilter::Info);

    let found = *(SKILL_GATE_TABLE_ADDRESS as *const [u8; 4]);
    if found != GATE_EXPECTED {
        error!("gate table at {SKILL_GATE_TABLE_ADDRESS:#010x} reads {found:?}, expected {GATE_EXPECTED:?} - not patching");
        return 1;
    }
    if let Err(step) = write_readonly(SKILL_GATE_TABLE_ADDRESS, &GATE_PATCHED) {
        error!("could not raise the gate table: {step}");
        return 2;
    }

    match hook_call_rel32(INITIALIZER_CALL_OFFSET, initializer_hook as usize as u32) {
        Ok(hook) => INITIALIZER_HOOK.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
        Err(_) => {
            error!("failed to hook the auto-trader initializer call");
            return 3;
        }
    }

    info!("captain skill cap gate fixed: gate table {GATE_EXPECTED:?} -> {GATE_PATCHED:?}, new records clamped at creation");
    0
}

/// `0x004FDF50(record, index, captain_flag)` - the record initializer, thiscall with two
/// stack arguments. It rolls the three skills under a 600-point total budget without
/// consulting the per-slot caps, so we clamp them afterwards.
///
/// The index arrives as an argument, which is exactly what the caps are derived from, so
/// no address arithmetic is needed. `captain_flag` is the game's own captain/pirate
/// discriminator (non-zero = the `% 11` captain path) and decides one thing here: whether
/// the wage the record already carries was computed from the skills we just clamped.
///
/// - pirate (`captain_flag == 0`): the initializer computed the wage inline at
///   `0x004FE104`, *before* this hook ran, so a clamp leaves it overstated - recompute.
///   (The tavern spawn at `0x00526A87` runs `0x004FE190` once more for this kind after
///   the allocator returns - same formula, now from the clamped skills, so the two
///   agree; it is captains that site skips, not pirates.)
/// - captain (`captain_flag != 0`): the initializer wrote `0`, which a clamp cannot make
///   stale, and it stays 0 until something computes a real wage after the allocator
///   returns - e.g. the create-onto-a-ship path at `0x0050AEE7` - which by then sees
///   clamped skills. Recomputing here would wrongly replace the 0 with a hire-time wage.
#[no_mangle]
unsafe extern "thiscall" fn initializer_hook(record: u32, index: u32, captain_flag: u32) {
    let original: extern "thiscall" fn(u32, u32, u32) = mem::transmute((*INITIALIZER_HOOK.load(Ordering::SeqCst)).old_absolute);
    original(record, index, captain_flag);

    let trader = AutoTraderPtr::new(record);
    let (navigation_cap, trade_cap, combat_cap) = skill_caps(index as u16);
    let navigation = trader.get_navigation_skill();
    let trade = trader.get_trade_skill();
    let combat = trader.get_combat_skill();

    let mut clamped = Vec::new();
    if navigation > navigation_cap {
        trader.set_navigation_skill(navigation_cap);
        clamped.push(format!("nav {navigation}->{navigation_cap}"));
    }
    if trade > trade_cap {
        trader.set_trade_skill(trade_cap);
        clamped.push(format!("trade {trade}->{trade_cap}"));
    }
    if combat > combat_cap {
        trader.set_combat_skill(combat_cap);
        clamped.push(format!("combat {combat}->{combat_cap}"));
    }
    if clamped.is_empty() {
        return;
    }

    let kind = if captain_flag != 0 { "captain" } else { "pirate" };
    // The pirate path already wrote a wage derived from the unclamped skills.
    if captain_flag == 0 {
        trader.recompute_daily_wage();
    }
    info!("clamped {kind} trader {index}: {} | caps {navigation_cap}/{trade_cap}/{combat_cap}", clamped.join(" "));
}
