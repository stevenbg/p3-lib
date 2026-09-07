//! Temporary instrumentation: what a battle looks like from inside the per-ship step.
//!
//! Written to answer why a town attack still succeeds while the mod reports forcing the
//! ships to flee. It logs to `_battle_flee.log` in the game folder, appended and never
//! truncated, and comes out once the question is settled.
//!
//! Three kinds of line:
//! - `battle <addr> FIRST SEEN` plus hex windows of the battle object, once per battle, so
//!   a town battle can be diffed against a sea one to find what marks it;
//! - `snap` blocks, at most one per [SNAPSHOT_INTERVAL_MS], walking the whole pool: every
//!   battle, its aggregates and outcome fields, and every ship object in it with its flags;
//! - `step` lines whenever a step changes a ship's flags or hands it a fresh grace period.
//!   after, so a flag the game clears again is visible.
//!
//! Everything is capped by [MAX_LINES] so a long session cannot fill the disk, and every
//! read is guarded - a battle's arrays are engine-owned and a town battle may not populate
//! them the way a sea battle does.

use std::{
    io::Write,
    sync::atomic::{AtomicU32, AtomicU64, Ordering},
    sync::Mutex,
};

use p3_api::{
    battle::{BattlePoolPtr, BattlePtr, BattleShipPtr},
    memory::is_readable,
    operations::OPERATIONS_PTR,
    ships::ShipsPtr,
};

const LOG_FILE: &str = "_battle_flee.log";
/// At most one pool snapshot this often; the step runs per ship per tick.
const SNAPSHOT_INTERVAL_MS: u64 = 500;
/// Total lines this probe will ever write.
const MAX_LINES: u32 = 200_000;
/// A sanity ceiling on the battle's own ship count, in case the field is nonsense.
const SHIP_SLOTS_MAX: u32 = 64;
/// Step transitions are logged at most this often, so they cannot drown the operation
/// dumps; the first [STEP_LINES_FREE] are always written.
const STEP_INTERVAL_MS: u64 = 100;
const STEP_LINES_FREE: u32 = 300;
/// Windows of the battle object dumped on first sight: the header, the status area, the
/// slot / step area, and the aggregates and outcome.
const DUMP_WINDOWS: [(u32, usize); 4] = [(0x000, 0x40), (0x150, 0x20), (0x650, 0x40), (0xA80, 0x50)];

static LINES: AtomicU32 = AtomicU32::new(0);
static LAST_SNAPSHOT_MS: AtomicU64 = AtomicU64::new(0);
static LAST_STEP_MS: AtomicU64 = AtomicU64::new(0);
static SEEN_BATTLES: Mutex<Vec<u32>> = Mutex::new(Vec::new());

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn write_line(line: &str) {
    if LINES.fetch_add(1, Ordering::Relaxed) >= MAX_LINES {
        return;
    }
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(LOG_FILE) {
        let _ = writeln!(file, "{line}");
    }
}

/// The `+0x12A` flags spelled out, so a log line says what a ship is rather than a number.
/// The two "out" bits are named as they are known: `0x02` is verified as leaving the
/// battle, `0x04` has no located writer.
fn flag_names(flags: u8) -> String {
    let mut parts = Vec::new();
    for (bit, name) in [
        (0x01u8, "fleeing"),
        (0x02, "withdrawn"),
        (0x04, "sunk"),
        (0x08, "grappled"),
        (0x10, "firing"),
        (0x20, "disengaged"),
        (0x40, "bit6"),
        (0x80, "ordermod"),
    ] {
        if flags & bit != 0 {
            parts.push(name);
        }
    }
    if parts.is_empty() {
        "none".into()
    } else {
        parts.join("|")
    }
}

unsafe fn hex(address: u32, len: usize) -> String {
    if !is_readable(address, len) {
        return "<unreadable>".into();
    }
    std::slice::from_raw_parts(address as *const u8, len)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Called from the step hook for every ship, before the mod decides anything.
pub unsafe fn observe(object: u32) {
    let ship_object = BattleShipPtr::new(object);
    if let Some(battle) = ship_object.get_battle() {
        first_seen(&battle);
    }
    let now = now_ms();
    let last = LAST_SNAPSHOT_MS.load(Ordering::Relaxed);
    if now.saturating_sub(last) < SNAPSHOT_INTERVAL_MS {
        return;
    }
    if LAST_SNAPSHOT_MS.compare_exchange(last, now, Ordering::SeqCst, Ordering::SeqCst).is_err() {
        return;
    }
    snapshot();
}

/// The hex windows and the vtable of a battle, once per battle object.
unsafe fn first_seen(battle: &BattlePtr) {
    {
        let mut seen = SEEN_BATTLES.lock().unwrap();
        if seen.contains(&battle.address) {
            return;
        }
        seen.push(battle.address);
    }
    let vtable: u32 = if is_readable(battle.address, 4) { *(battle.address as *const u32) } else { 0 };
    let kind = match vtable {
        BattlePtr::VTABLE_INTERACTIVE => "interactive",
        BattlePtr::VTABLE_RECORD => "record",
        _ => "unknown",
    };
    write_line(&format!(
        "battle {:#010x} FIRST SEEN vtable {vtable:#010x} ({kind}) slot {} marker {} status {:#06x}",
        battle.address,
        battle.get_slot(),
        battle.get_marker(),
        battle.get_status()
    ));
    for (offset, len) in DUMP_WINDOWS {
        write_line(&format!("  +{offset:#05x}: {}", hex(battle.address + offset, len)));
    }
}

/// Every battle in the pool, its fields and its ships.
pub unsafe fn snapshot() {
    let pool = BattlePoolPtr::new();
    let player = OPERATIONS_PTR.get_player_merchant_index();
    write_line(&format!(
        "snap t={} battles={} player_slot={} player_merchant={player}",
        now_ms(),
        pool.count(),
        pool.player_battle_slot()
    ));
    for slot in 0..=0xFEu8 {
        let Some(battle) = pool.get_battle(slot) else { continue };
        if !is_readable(battle.address, 0xAD0) {
            write_line(&format!("  slot {slot}: {:#010x} <unreadable>", battle.address));
            continue;
        }
        write_line(&format!(
            "  slot {slot} {:#010x} own_slot={} player={} ships={} marker={} status={:#06x} step={} arg={} latch={} outcome={:#04x}/ship {}",
            battle.address,
            battle.get_slot(),
            battle.is_player_battle(),
            battle.get_ship_count(),
            battle.get_marker(),
            battle.get_status(),
            battle.get_step_counter(),
            battle.get_step_argument(),
            battle.get_panic_latch(),
            battle.get_outcome_kind(),
            battle.get_outcome_ship()
        ));
        for side in 0..2u8 {
            write_line(&format!(
                "    side {side}: head={} gunnery={} max_crew={} boarding={} armament={} speed={} scale={} mean=({},{})",
                battle.get_side_head(side),
                battle.get_side_gunnery(side),
                battle.get_side_max_crew(side),
                battle.get_side_max_boarding_power(side),
                battle.get_side_max_armament(side),
                battle.get_side_max_speed(side),
                battle.get_side_scale(side),
                battle.get_side_mean_x(side),
                battle.get_side_mean_y(side)
            ));
        }
        let count = battle.get_ship_count().min(SHIP_SLOTS_MAX);
        for index in 0..count as u16 {
            let Some(ship_object) = battle.get_ship_object(index) else { continue };
            if !is_readable(ship_object.address, 0x150) {
                write_line(&format!("    ship[{index}] {:#010x} <unreadable>", ship_object.address));
                continue;
            }
            let world_index = ship_object.get_ship_index();
            let world = u16::try_from(world_index).ok().and_then(|i| ShipsPtr::new().get_ship(i));
            let who = match world {
                Some(ship) => format!(
                    "'{}' merchant={} status={} hull={}/{}",
                    ship.get_name(),
                    ship.get_merchant_index(),
                    ship.get_status(),
                    ship.get_current_health(),
                    ship.get_max_health()
                ),
                None => "<no world ship>".into(),
            };
            write_line(&format!(
                "    ship[{index}] {:#010x} world={world_index} side={} flags={:#04x} [{}] grace={} order={} target={} pos=({},{}) head={} sail={}/{} hull0={} crew0={} {who}",
                ship_object.address,
                ship_object.get_side(),
                ship_object.get_flags(),
                flag_names(ship_object.get_flags()),
                ship_object.get_grace_counter(),
                ship_object.get_order_mode(),
                ship_object.get_target(),
                ship_object.get_x(),
                ship_object.get_y(),
                ship_object.get_heading(),
                ship_object.get_sail_setting(),
                ship_object.get_applied_sail_setting(),
                ship_object.get_hull_at_start(),
                ship_object.get_crew_at_start()
            ));
        }
    }
}

/// What the AI itself did to a ship across one step: the flags this mod left behind on the
/// previous step (`entry`) against what the step decided (`exit`). A flee flag present on
/// entry and gone on exit is the AI overwriting the order, which is the difference between
/// setting the flag and the ship actually running.
pub unsafe fn note_step(object: u32, entry: u8, exit: u8, grace_entry: u8) {
    let ship_object = BattleShipPtr::new(object);
    let grace_exit = ship_object.get_grace_counter();
    // A counter that goes up is the game handing the ship a fresh grace period, which is
    // the mechanism worth catching; a counter that merely ticks down is routine.
    let grace_rose = grace_exit > grace_entry;
    if entry == exit && !grace_rose {
        return;
    }
    if LINES.load(Ordering::Relaxed) > STEP_LINES_FREE {
        let now = now_ms();
        let last = LAST_STEP_MS.load(Ordering::Relaxed);
        if now.saturating_sub(last) < STEP_INTERVAL_MS || LAST_STEP_MS.compare_exchange(last, now, Ordering::SeqCst, Ordering::SeqCst).is_err() {
            return;
        }
    }
    let verdict = match (entry & BattleShipPtr::FLAG_FLEEING != 0, exit & BattleShipPtr::FLAG_FLEEING != 0) {
        (true, false) => "AI CLEARED the flee",
        (false, true) => "AI chose to flee",
        (true, true) => "flee kept",
        (false, false) => "no flee either side",
    };
    write_line(&format!(
        "step {:#010x} entry={entry:#04x} [{}] exit={exit:#04x} [{}] {verdict} grace {grace_entry}->{grace_exit}{} order={} target={} sail={}/{} head={} pos=({},{})",
        object,
        flag_names(entry),
        flag_names(exit),
        if grace_rose { " ROSE" } else { "" },
        ship_object.get_order_mode(),
        ship_object.get_target(),
        ship_object.get_sail_setting(),
        ship_object.get_applied_sail_setting(),
        ship_object.get_heading(),
        ship_object.get_x(),
        ship_object.get_y()
    ));
}

/// Every battle-ship object of a battle, as one line each - used either side of an
/// operation so its effect is a diff rather than a guess.
pub unsafe fn dump_battle_ships(battle: &BattlePtr, tag: &str) {
    let count = battle.get_ship_count().min(SHIP_SLOTS_MAX);
    for index in 0..count as u16 {
        let Some(ship_object) = battle.get_ship_object(index) else { continue };
        if !is_readable(ship_object.address, 0x150) {
            continue;
        }
        let world_index = ship_object.get_ship_index();
        let world = u16::try_from(world_index).ok().and_then(|i| ShipsPtr::new().get_ship(i));
        let who = match world {
            Some(ship) => format!(
                "'{}' status={} hull={}/{}",
                ship.get_name(),
                ship.get_status(),
                ship.get_current_health(),
                ship.get_max_health()
            ),
            None => "<no world ship>".into(),
        };
        write_line(&format!(
            "  {tag} ship[{index}] {:#010x} side={} flags={:#04x} [{}] grace={} order={} target={} sail={}/{} head={} pos=({},{}) {who}",
            ship_object.address,
            ship_object.get_side(),
            ship_object.get_flags(),
            flag_names(ship_object.get_flags()),
            ship_object.get_grace_counter(),
            ship_object.get_order_mode(),
            ship_object.get_target(),
            ship_object.get_sail_setting(),
            ship_object.get_applied_sail_setting(),
            ship_object.get_heading(),
            ship_object.get_x(),
            ship_object.get_y(),
        ));
        write_line(&format!("  {tag} ship[{index}] raw +0x00: {}", hex(ship_object.address, 0x30)));
        write_line(&format!("  {tag} ship[{index}] raw +0x120: {}", hex(ship_object.address + 0x120, 0x30)));
    }
}

/// A battle order operation as it is drained, before the switch runs it. Returns the battle
/// it addresses so the caller can dump the same ships afterwards.
pub unsafe fn note_operation_before(op: u32) -> Option<BattlePtr> {
    if !is_readable(op, 0x14) {
        return None;
    }
    let opcode = *(op as *const u32);
    if !(0x93..=0x9B).contains(&opcode) {
        return None;
    }
    let raw = hex(op, 0x14);
    let slot = *((op + 4) as *const u8);
    let battle = BattlePoolPtr::new().get_battle(slot);
    write_line(&format!(
        "OP {opcode:#04x} slot={slot} battle={} raw {raw}",
        battle.map(|b| format!("{:#010x}", b.address)).unwrap_or_else(|| "<none>".into())
    ));
    if let Some(battle) = battle {
        write_line(&format!(
            "  OP battle latch={} status={:#06x} outcome={:#04x}/ship {} side1head={}",
            battle.get_panic_latch(),
            battle.get_status(),
            battle.get_outcome_kind(),
            battle.get_outcome_ship(),
            battle.get_side_head(1)
        ));
        dump_battle_ships(&battle, "BEFORE");
    }
    battle
}

/// The same battle's ships after the switch has run the operation.
pub unsafe fn note_operation_after(battle: &BattlePtr) {
    dump_battle_ships(battle, "AFTER");
    write_line(&format!(
        "  OP done latch={} status={:#06x} outcome={:#04x}/ship {}",
        battle.get_panic_latch(),
        battle.get_status(),
        battle.get_outcome_kind(),
        battle.get_outcome_ship()
    ));
}

/// The ship objects that have already had their one honest step, keyed by address - a
/// battle-ship object is a fresh allocation per battle, so this is per ship per battle.
static SAMPLED: Mutex<Vec<u32>> = Mutex::new(Vec::new());

/// Whether this ship should be allowed **one** step at its real courage, to record what the
/// AI decides without us. Only once the grace counter has run out, because while it is up
/// the evaluation is skipped and there is no decision to observe. One step is ~27 ms and
/// the AI re-decides every step, so the sample costs nothing and settles the only question
/// a run needs to answer: whether the mod changed the outcome or the ship would have fled
/// regardless.
pub unsafe fn wants_verdict_sample(object: u32) -> bool {
    if BattleShipPtr::new(object).get_grace_counter() != 0 {
        return false;
    }
    let mut sampled = SAMPLED.lock().unwrap();
    if sampled.contains(&object) {
        return false;
    }
    sampled.push(object);
    true
}

/// What the AI decided on that honest step, and therefore whether the mod matters here.
pub unsafe fn note_verdict(object: u32, name: &str, battle: &BattlePtr, scale: u8) {
    let ship_object = BattleShipPtr::new(object);
    let side = ship_object.get_side();
    let fled = ship_object.get_flags() & BattleShipPtr::FLAG_FLEEING != 0;
    write_line(&format!(
        "VERDICT '{name}' battle {:#010x} side {side} scale {scale}: the AI chose to {} on its own - {}",
        battle.address,
        if fled { "FLEE" } else { "FIGHT" },
        if fled {
            "it would flee without the mod, this run proves nothing about the mod"
        } else {
            "IT WOULD HAVE FOUGHT, so the mod is what makes it flee"
        }
    ));
    write_line(&format!(
        "  VERDICT inputs: own gunnery {} boarding {} crew {}, enemy gunnery {} boarding {} crew {}, hull {}",
        battle.get_side_gunnery(side),
        battle.get_side_max_boarding_power(side),
        battle.get_side_max_crew(side),
        battle.get_side_gunnery(1 - side),
        battle.get_side_max_boarding_power(1 - side),
        battle.get_side_max_crew(1 - side),
        ShipsPtr::new()
            .get_ship(ship_object.get_ship_index() as u16)
            .map(|s| format!("{}/{}", s.get_current_health(), s.get_max_health()))
            .unwrap_or_else(|| "?".into()),
    ));
}
