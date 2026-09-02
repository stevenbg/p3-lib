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
//! fully frozen - and it is why the log below exists while the fix is being judged.
//!
//! Background: `.claude/notes/todo/captain-navigation-cap-bug.md` and
//! `.claude/notes/done/captain-experience.md`.
use std::{
    io::Write,
    mem,
    sync::atomic::{AtomicPtr, AtomicU32, Ordering},
};

use hooklet::windows::x86::{hook_call_rel32, CallRel32Hook};
use log::{error, info, warn};
use p3_api::{
    auto_trader::{skill_caps, AutoTraderPtr, SKILL_CAPS, SKILL_GATE_TABLE_ADDRESS},
    game_world::GAME_WORLD_PTR,
    memory::write_readonly,
    ships::ShipsPtr,
};

/// What the gate table must contain before we touch it, and what we put there. Verifying
/// first means a different game build fails loudly instead of corrupting `.rdata`.
const GATE_EXPECTED: [u8; 4] = SKILL_CAPS;
const GATE_PATCHED: [u8; 4] = [250; 4];

/// `call 0x004FDF50` at `0x00509897`, the record initializer's only call site, inside the
/// only allocator (`0x005097C0`). Module-relative, as `hook_call_rel32` takes it.
const INITIALIZER_CALL_OFFSET: u32 = 0x0010_9897;
/// `call 0x00538A80` at `0x00535973`, the skill-gain handler's only call site. Hooked for
/// observation only - the hook changes nothing.
const GAIN_CALL_OFFSET: u32 = 0x0013_5973;

/// The operation queue's pending count (`queue+0x56` on `0x006DF2F0`). The scan gives
/// itself `0x34 - this` enqueues per run, so this is the number that would have to climb
/// for patch 1's extra no-op operations to start costing anything.
const PENDING_OPERATIONS: *const u16 = 0x006D_F346 as _;
const PENDING_OPERATIONS_CEILING: u16 = 0x34;

/// Operation `0x12`'s payload: the ship whose captain gains, and the two gain fields
/// (`+0xC` is applied to trade *and* combat - the handler reads it twice).
const OPERATION_SHIP_INDEX: u32 = 0x4;
const OPERATION_NAVIGATION_GAIN: u32 = 0x8;
const OPERATION_TRADE_COMBAT_GAIN: u32 = 0xC;

/// In the game folder, next to `_crash_report.txt`. Never truncated, so runs accumulate.
const LOG_FILE: &str = "_captain_skill_gate.log";

/// One summary line per this many gain events, so a long soak stays readable.
const SUMMARY_EVERY: u32 = 100;

static INITIALIZER_HOOK: AtomicPtr<CallRel32Hook> = AtomicPtr::new(std::ptr::null_mut());
static GAIN_HOOK: AtomicPtr<CallRel32Hook> = AtomicPtr::new(std::ptr::null_mut());

static GAINS_SEEN: AtomicU32 = AtomicU32::new(0);
static SKILLS_RAISED: AtomicU32 = AtomicU32::new(0);
static SKILLS_PRUNED: AtomicU32 = AtomicU32::new(0);
static CREATIONS_CLAMPED: AtomicU32 = AtomicU32::new(0);
static PEAK_PENDING: AtomicU32 = AtomicU32::new(0);

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
    match hook_call_rel32(GAIN_CALL_OFFSET, skill_gain_observer as usize as u32) {
        Ok(hook) => GAIN_HOOK.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
        Err(_) => {
            error!("failed to hook the skill gain handler call for observation");
            return 4;
        }
    }

    info!(
        "captain skill cap gate fixed: gate table {GATE_EXPECTED:?} -> {GATE_PATCHED:?}, new records clamped at creation, gains logged to {LOG_FILE}"
    );
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
    let original: extern "thiscall" fn(u32, u32, u32) =
        mem::transmute((*INITIALIZER_HOOK.load(Ordering::SeqCst)).old_absolute);
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

    let count = CREATIONS_CLAMPED.fetch_add(1, Ordering::Relaxed) + 1;
    let line = format!(
        "clamped {kind} #{count}: trader {index} {} {} | caps {navigation_cap}/{trade_cap}/{combat_cap} | wage {}",
        trader.get_name().unwrap_or_else(|| "<unnamed>".into()),
        clamped.join(" "),
        trader.get_daily_wage(),
    );
    warn!("{line}");
    append_log_line(&line);
}

/// `0x00538A80(operation)` - the skill gain handler, thiscall, no stack arguments.
///
/// **Observation only**: it calls through unchanged and exists to show the fix working
/// while it is being judged. It comes out, with the file logging, once it has.
///
/// Records are resolved the way the handler itself does - operation `+0x4` ship index,
/// `ship+0x42` captain index, then the auto-trader array - but through p3-api's
/// bounds-checked accessors, which check the same counts the handler checks. Only
/// human-owned ships are reported: AI captains take a flat `+8` every eligible round and
/// would swamp the file, and ownerless pirate ships grow through the hideout award rather
/// than the gate this mod fixes.
#[no_mangle]
unsafe extern "thiscall" fn skill_gain_observer(operation: u32) {
    let original: extern "thiscall" fn(u32) = mem::transmute((*GAIN_HOOK.load(Ordering::SeqCst)).old_absolute);

    let pending = *PENDING_OPERATIONS;
    PEAK_PENDING.fetch_max(pending as u32, Ordering::Relaxed);
    let seen = GAINS_SEEN.fetch_add(1, Ordering::Relaxed) + 1;

    let ship_index = *((operation + OPERATION_SHIP_INDEX) as *const u32);
    let navigation_gain = *((operation + OPERATION_NAVIGATION_GAIN) as *const u32);
    let trade_combat_gain = *((operation + OPERATION_TRADE_COMBAT_GAIN) as *const u32);

    let ships = ShipsPtr::new();
    let subject = u16::try_from(ship_index)
        .ok()
        .and_then(|index| ships.get_ship(index))
        .and_then(|ship| {
            let captain_index = ship.get_captain_index();
            ships
                .get_auto_trader(captain_index)
                .map(|trader| (ship, captain_index, trader))
        })
        .filter(|(ship, _, _)| is_human_owned(ship.get_merchant_index()));

    let Some((_, captain_index, trader)) = subject else {
        original(operation);
        log_summary_if_due(seen);
        return;
    };

    let before = (
        trader.get_navigation_skill(),
        trader.get_trade_skill(),
        trader.get_combat_skill(),
    );
    original(operation);
    let after = (
        trader.get_navigation_skill(),
        trader.get_trade_skill(),
        trader.get_combat_skill(),
    );

    if after != before {
        let (navigation_cap, trade_cap, combat_cap) = skill_caps(captain_index);
        let mut moves = Vec::new();
        for (name, old, new) in [
            ("nav", before.0, after.0),
            ("trade", before.1, after.1),
            ("combat", before.2, after.2),
        ] {
            if new == old {
                continue;
            }
            if new > old {
                SKILLS_RAISED.fetch_add(1, Ordering::Relaxed);
                moves.push(format!("{name} {old}->{new} (+{})", new - old));
            } else {
                SKILLS_PRUNED.fetch_add(1, Ordering::Relaxed);
                moves.push(format!("{name} {old}->{new} (PRUNED -{})", old - new));
            }
        }
        let line = format!(
            "gain #{seen}: trader {captain_index} {} ship {ship_index} | {} | caps {navigation_cap}/{trade_cap}/{combat_cap} | rolled nav {navigation_gain} trade&combat {trade_combat_gain} | pending {pending}/{PENDING_OPERATIONS_CEILING}",
            trader.get_name().unwrap_or_else(|| "<unnamed>".into()),
            moves.join(" "),
        );
        warn!("{line}");
        append_log_line(&line);
    }

    log_summary_if_due(seen);
}

/// A human merchant owns the ship: control word 0. `0xFF` and anything at or past the
/// merchant count is "nobody" - an ownerless pirate ship, which is how the hideout award
/// and hired pirates arrive here.
unsafe fn is_human_owned(merchant_index: u8) -> bool {
    let merchant_index = merchant_index as u16;
    merchant_index < GAME_WORLD_PTR.get_merchants_count()
        && GAME_WORLD_PTR.get_merchant(merchant_index).get_control_word() == 0
}

fn log_summary_if_due(seen: u32) {
    if seen % SUMMARY_EVERY != 0 {
        return;
    }
    let line = format!(
        "summary: {seen} gains seen | {} skills raised | {} pruned | {} creations clamped | peak pending {}/{PENDING_OPERATIONS_CEILING}",
        SKILLS_RAISED.load(Ordering::Relaxed),
        SKILLS_PRUNED.load(Ordering::Relaxed),
        CREATIONS_CLAMPED.load(Ordering::Relaxed),
        PEAK_PENDING.load(Ordering::Relaxed),
    );
    info!("{line}");
    append_log_line(&line);
}

/// Temporary evidence file. Write failures are swallowed: logging must never be the thing
/// that breaks a frame.
fn append_log_line(line: &str) {
    let unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(LOG_FILE) {
        let _ = writeln!(file, "[unix {unix}] {line}");
    }
}
