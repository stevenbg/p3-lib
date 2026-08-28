//! The debug keys: throwaway in-game probes for whatever is being reverse-engineered
//! right now, moved here from mod-auto-supply so the gameplay mod and the debugging
//! toolbox stay separate. F9 (with modifiers) and F10 are reserved for these; the
//! probe bodies are rewritten per investigation.
//!
//! Keys are dispatched through the shared registry (`hotkeys.dll`); without it the
//! probes are inert. All handlers decline (return 0), so the game still sees the
//! keys.
//!
//! Debug builds only: the module is `#[cfg(debug_assertions)]`-gated in lib.rs, so
//! `--release` builds carry none of this.
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, Ordering};
use std::sync::Mutex;
use std::mem;

use hooklet::windows::x86::{hook_call_rel32, CallRel32Hook};
use log::{debug, error, info};
use num_traits::FromPrimitive;
use p3_api::{
    auto_trader::skill_caps,
    data::{enums::WareId, office::OFFICE_SIZE, p3_ptr::P3Pointer},
    game_world::{GAME_WORLD_PTR, TICKS_PER_YEAR},
    hotkeys::{HotkeysApi, MOD_ALT, MOD_CTRL, MOD_SHIFT},
    operations::OPERATIONS_PTR,
    scheduled_tasks::{
        scheduled_task::{SCHEDULED_TASK_OPCODE_TEN_DAY_UPDATE, SCHEDULED_TASK_OPCODE_UNFREEZE_PORT},
        SCHEDULED_TASKS_PTR,
    },
    ship::SHIP_SIZE,
    ships::ShipsPtr,
    town::get_town_name,
    ui::{ui_ship_panel::UIShipPanelPtr, ui_trading_office_window::UITradingOfficeWindowPtr},
};
use windows::Win32::UI::Input::KeyboardAndMouse::{VK_F10, VK_F9};

const DEBUG_PROBE1_KEY: u32 = VK_F9.0 as u32;
const ROUTE_DUMP_KEY: u32 = VK_F10.0 as u32;

/// The registry binding, null when hotkeys.dll is unavailable (probes inert).
static HOTKEYS: AtomicPtr<HotkeysApi> = AtomicPtr::new(std::ptr::null_mut());
const OWNER: &std::ffi::CStr = c"crash-reporter debug probes";

const PROBE_KEYS: [(u32, u32); 11] = [
    (DEBUG_PROBE1_KEY, 0),
    (DEBUG_PROBE1_KEY, MOD_CTRL),
    (DEBUG_PROBE1_KEY, MOD_SHIFT),
    (DEBUG_PROBE1_KEY, MOD_ALT),
    // The operation logger: the one permanent tool here, rather than a throwaway.
    (DEBUG_PROBE1_KEY, MOD_CTRL | MOD_SHIFT),
    (ROUTE_DUMP_KEY, 0),
    (ROUTE_DUMP_KEY, MOD_CTRL),
    (ROUTE_DUMP_KEY, MOD_SHIFT),
    // The captain-retirement hang probe. Exact-match modifiers, so these three never
    // collide with each other or with plain F10.
    (ROUTE_DUMP_KEY, MOD_ALT),
    (ROUTE_DUMP_KEY, MOD_ALT | MOD_CTRL),
    (ROUTE_DUMP_KEY, MOD_ALT | MOD_SHIFT),
];

/// Bind the registry and register the probe keys; called from start(). A missing
/// registry only disables the probes, never the crash reporting.
pub unsafe fn install() {
    match HotkeysApi::bind() {
        Ok(api) => {
            HOTKEYS.store(Box::into_raw(Box::new(api)), Ordering::SeqCst);
            let api = &*HOTKEYS.load(Ordering::SeqCst);
            for (vk, mods) in PROBE_KEYS {
                api.register(OWNER, vk, mods, probe_hotkeys);
            }
        }
        Err(reason) => log::warn!("hotkeys registry unavailable ({reason}) - debug probes inert"),
    }
}

/// Session-global; the registrations live for the whole process, so the handles are
/// never stored. Handlers decline so the game keeps seeing F9/F10.
#[no_mangle]
unsafe extern "C" fn probe_hotkeys(vk: u32, mods: u32) -> u32 {
    match (vk, mods) {
        (DEBUG_PROBE1_KEY, MOD_CTRL) => toggle_probe1_timeline(),
        (DEBUG_PROBE1_KEY, MOD_SHIFT) => debug_probe1_ship(),
        (DEBUG_PROBE1_KEY, MOD_ALT) => debug_probe_administrators(),
        (DEBUG_PROBE1_KEY, m) if m == MOD_CTRL | MOD_SHIFT => install_op_logger(),
        (DEBUG_PROBE1_KEY, 0) => debug_probe1(),
        (ROUTE_DUMP_KEY, MOD_CTRL) => debug_probe_dialog_modes(),
        (ROUTE_DUMP_KEY, MOD_SHIFT) => debug_probe_ice(),
        (ROUTE_DUMP_KEY, MOD_ALT) => retire_probe_inspect(),
        (ROUTE_DUMP_KEY, m) if m == MOD_ALT | MOD_CTRL => retire_probe_benign(),
        (ROUTE_DUMP_KEY, m) if m == MOD_ALT | MOD_SHIFT => retire_probe_hang(),
        (ROUTE_DUMP_KEY, 0) => dump_ship_routes(),
        _ => {}
    }
    0
}

/// Post an in-game popup on the event ticker, mirrored to the debug log (a copy of
/// mod-auto-supply's helper - the probes' only shared dependencies were this and
/// [selected_ship_index]).
unsafe fn notify(text: &str) {
    info!("{text}");
    let latin1: Vec<u8> = text.chars().map(|c| if (c as u32) <= 0xff { c as u32 as u8 } else { b'?' }).collect();
    p3_api::ui::ui_notifications::UINotificationsPtr::new().post_event(&latin1);
}

unsafe fn selected_ship_index() -> Option<u16> {
    UIShipPanelPtr::new().get_selected_ship_index()
}

/// Global pool of applied route stops (VERIFIED in-game): 220-byte records in the .rou
/// stop layout, the first u16 is the next-stop index, the chain is circular. The
/// (lead) ship's ship+0x132 is the CURRENT stop pointer (advances as the route runs);
/// the stop carrying the 0x04 action marker is the route's logical first stop. A head
/// of 0 is ambiguous (pool[0] is a valid index), so empty ship slots also "dump".
const ROUTE_STOP_POOL_COUNT: *const u16 = 0x006dd72a as _;
const ROUTE_STOP_POOL: *const u32 = 0x006dd72c as _;
const ROUTE_STOP_SIZE: u32 = 220;
const SHIP_ROUTE_HEAD_OFFSET: u32 = 0x132;

/// The auto trade goods dialog ("Automatic maritime trading in ..."); the static holds
/// the object pointer.
const GOODS_DIALOG_PTR: *const u32 = 0x006cba74 as _;
/// Pool index of the stop the dialog is showing, -1 while it is closed.
const GOODS_DIALOG_STOP_OFFSET: u32 = 0xa4;
const GOODS_DIALOG_SHIP_OFFSET: u32 = 0xa8;
/// The dialog's per-ware order type, one i32 per ware id - only the 20 trade wares, the
/// dialog has no weapons rows, so reading 24 runs off the end into unrelated fields.
const GOODS_DIALOG_MODE_OFFSET: u32 = 0x4ae0;
const GOODS_DIALOG_MODE_COUNT: u32 = 20;
/// The dialog's block of ten per-ware control array pointers; each holds a `new[]` array
/// of 20 window-family widgets (constructor 0x004c6910, 0xe8 bytes each).
const GOODS_DIALOG_ARRAY_BLOCK_OFFSET: u32 = 0xf0;
/// The five of those the order type indexes to pick which per-ware sub window is
/// registered.
const GOODS_DIALOG_MODE_TABLE_OFFSET: u32 = 0xf4;

/// CTRL+F10 (THROWAWAY): dump the auto trade goods dialog's per-ware order type beside
/// the route stop record it was derived from, to settle which order type each mode value
/// means. Open the dialog on a stop, set a few wares to different order types with the
/// cycling button, then press: each line shows the dialog's mode value and, independently,
/// what the record's own price/amount signs say the order is.
unsafe fn debug_probe_dialog_modes() {
    let dialog = *GOODS_DIALOG_PTR;
    if dialog == 0 {
        notify("dialog probe: goods dialog not constructed");
        return;
    }
    let stop_index = *((dialog + GOODS_DIALOG_STOP_OFFSET) as *const i32);
    let ship_index = *((dialog + GOODS_DIALOG_SHIP_OFFSET) as *const i32);
    debug!("goods dialog at {dialog:#010x}: stop {stop_index}, ship {ship_index}");
    if stop_index < 0 {
        notify("dialog probe: dialog closed (stop -1) - open it on a stop first");
        return;
    }
    let pool = *ROUTE_STOP_POOL;
    let pool_count = *ROUTE_STOP_POOL_COUNT;
    if pool == 0 || stop_index as u16 >= pool_count {
        notify(&format!("dialog probe: stop {stop_index} outside the pool ({pool_count})"));
        return;
    }
    let stop = pool + stop_index as u32 * ROUTE_STOP_SIZE;

    // The dialog's ten per-ware control arrays. Five of them are what the order type
    // indexes (+0xf4 + mode*4); the rest are other columns of the row. Every element is a
    // window-family object, so element 0's rectangle (x +0x14, y +0x18, w +0x2c, h +0x30)
    // says which column each array draws - sort the lines by x and they read left to
    // right across the row.
    for slot in 0..10u32 {
        let offset = GOODS_DIALOG_ARRAY_BLOCK_OFFSET + slot * 4;
        let array = *((dialog + offset) as *const u32);
        let mode = match offset {
            o if (GOODS_DIALOG_MODE_TABLE_OFFSET..GOODS_DIALOG_MODE_TABLE_OFFSET + 20).contains(&o) => {
                format!("order type {}", (o - GOODS_DIALOG_MODE_TABLE_OFFSET) / 4)
            }
            _ => "not order-type".to_string(),
        };
        if array == 0 || !p3_api::memory::is_readable(array, 0x34) {
            debug!("  array +{offset:#05x}: {array:#010x} (unreadable) {mode}");
            continue;
        }
        debug!(
            "  array +{offset:#05x}: {array:#010x} elem0 x {:4} y {:4} w {:4} h {:4}  {mode}",
            *((array + 0x14) as *const i32),
            *((array + 0x18) as *const i32),
            *((array + 0x2c) as *const i32),
            *((array + 0x30) as *const i32),
        );
    }

    let order: Vec<String> = (0..24).map(|i| format!("{:02x}", *((stop + 4 + i) as *const u8))).collect();
    debug!("  order array: {}", order.join(" "));

    for ware in 0..24u32 {
        let mode = if ware < GOODS_DIALOG_MODE_COUNT {
            format!("{}", *((dialog + GOODS_DIALOG_MODE_OFFSET + ware * 4) as *const i32))
        } else {
            "-".to_string()
        };
        let price = *((stop + 28 + ware * 4) as *const i32);
        let amount = *((stop + 124 + ware * 4) as *const i32);
        // What the stop record itself encodes, derived without consulting the dialog.
        let record = match (price, amount) {
            (_, 0) => "no order",
            (0, a) if a > 0 => "load office -> ship",
            (0, _) => "unload ship -> office",
            (p, _) if p > 0 => "sell to town",
            _ => "buy from town",
        };
        let name = format!("{:?}", WareId::from_u32(ware).unwrap());
        debug!("  ware {ware:2} {name:12} mode {mode:>2}  price {price:11}  amount {amount:11}  record says {record}");
    }
    notify(&format!("dialog probe: stop {stop_index} modes dumped to DebugView"));
}

/// F10: walk every ship's route chain and dump the stops.
unsafe fn dump_ship_routes() {
    let pool = *ROUTE_STOP_POOL;
    let pool_count = *ROUTE_STOP_POOL_COUNT;
    debug!("route stop pool at {pool:#010x}, {pool_count} entries");
    if pool == 0 {
        return;
    }

    let ships = p3_api::ships::ShipsPtr::new();
    for ship_id in 0..ships.get_ships_size() {
        let Some(ship) = ships.get_ship(ship_id) else {
            continue;
        };
        let head = *((ship.address + SHIP_ROUTE_HEAD_OFFSET) as *const u16);
        if head >= pool_count {
            continue;
        }
        // The ship struct address is a candidate value for the map-selection global, if
        // the selection is stored as a pointer (scan for it in Cheat Engine, 4-byte hex,
        // while switching selected ships).
        debug!("ship {ship_id} at {:#010x} {:?}: route head {head}", ship.address, ship.get_name());

        let mut index = head;
        for n in 0..32 {
            let stop = pool + index as u32 * ROUTE_STOP_SIZE;
            let next = *(stop as *const u16);
            let town_index = *((stop + 2) as *const u8);
            let action = *((stop + 3) as *const u8);
            let town = get_town_name(town_index).unwrap_or_else(|| format!("<{town_index:#04x}>"));
            let mut ops = Vec::new();
            for i in 0..24usize {
                let price = *((stop + 28 + i as u32 * 4) as *const i32);
                let amount = *((stop + 124 + i as u32 * 4) as *const i32);
                if price != 0 || amount != 0 {
                    ops.push(format!("{:?} p{price} a{amount}", WareId::from_usize(i).unwrap()));
                }
            }
            debug!(
                "  stop {n}: pool[{index}] town {town}, action {action:#04x}, next {next}, ops [{}]",
                ops.join(", ")
            );
            index = next;
            if index == head || index >= pool_count {
                break;
            }
        }
    }
}

/// F9 (THROWAWAY): the captain-experience census, for
/// `.claude/notes/done/captain-experience.md`. Dumps to DebugView and to `_probe1.log`
/// in the game folder - always appended, never truncated, so presses days or years
/// apart sit in one file and diff directly.
///
/// Restored to verify `mod-fix-captain-skill-cap-gate`: with that mod loaded, `GATED OUT`
/// on the player's captains should fall to near zero, a captain whose navigation cap is
/// below his trade or combat cap should keep gaining past it, and newly created records
/// should never be `OVER` their caps.
///
/// Every skill byte in the game is written by operation `0x12` (`0x00538A80`), which
/// clamps each skill to a ceiling taken from bits of the record's **array index**, and
/// the only producer that runs continuously is the ten-day scan `0x004DCEA0`. So the
/// dump prints, per record, the three skills against the three ceilings
/// [p3_api::auto_trader::skill_caps] derives, where the record sits (tavern, ship,
/// office) and who owns it - the scan gives a human owner's captains a random 0..50 on
/// one skill and an AI owner's a flat +8, told apart by `merchant+0x8`.
///
/// What to check in a dump:
///
/// - **no skill above its cap.** The handler writes the cap unconditionally when a gain
///   would pass it, so an `OVER` row is a record no gain event has ever touched.
/// - **`trade - combat` constant** for the same captain across two presses a year apart:
///   the handler reads one payload field for both skills.
/// - **administrator trade skills exact multiples of 43** - their own path adds exactly
///   one level at a time and stops at 215.
/// - **the round counter between 0 and 31**, consistent with the day of the year: it is
///   reset below day 10 and the scan returns early above `0x1F`.
unsafe fn debug_probe1() {
    let mut out: Vec<String> = Vec::new();
    let ships = ShipsPtr::new();
    let traders = ships.get_auto_traders_size();
    let ship_count = ships.get_ships_size();
    let merchants = GAME_WORLD_PTR.get_merchants_count();
    let now = GAME_WORLD_PTR.get_game_time_raw();
    let local: u32 = *(0x006DFC14 as *const u32);
    let press = PROBE1_PRESSES.fetch_add(1, Ordering::SeqCst);

    // Which save this block came from. The loaded file name is not kept anywhere the mod
    // can read, so the block is keyed by the player himself plus a hash of the world's
    // town list - enough to group blocks by save, and the tick orders them within one.
    // An optional `_probe1_save.txt` in the game folder adds a label of your own.
    let label = std::fs::read_to_string("_probe1_save.txt")
        .map(|text| text.lines().next().unwrap_or("").trim().to_string())
        .unwrap_or_default();
    // Which save folder the game writes to: the path builder at 0x005473A6 picks
    // `Save\Kam`, `Save\Ein` or `Save\Mehr` off this byte of the setup object.
    let mode = *((*(0x006CC3E8 as *const u32) + 0xd) as *const u8);
    let campaign = match mode {
        5 => "Kam",
        3 => "Ein",
        0..=2 => "Mehr",
        _ => "?",
    };
    // FNV-1a over the town id list at `game_world+0x18`, which world generation fills:
    // constant within a save, different between worlds.
    let world = GAME_WORLD_PTR.get::<[u8; 40]>(0x18).iter().fold(0x811c_9dc5u32, |hash, &byte| {
        (hash ^ byte as u32).wrapping_mul(0x0100_0193)
    });
    let player = GAME_WORLD_PTR.get_merchant(local as u16);

    out.push(format!(
        "=== press {press} | save: {} campaign {campaign}({mode}) world {world:#010x} | tick {now} year {} day-of-year {} ({}.{}) ===",
        if label.is_empty() { "<no _probe1_save.txt>".to_string() } else { format!("\"{label}\"") },
        GAME_WORLD_PTR.get_year(),
        GAME_WORLD_PTR.get_day_of_year(),
        GAME_WORLD_PTR.get_day_of_month(),
        GAME_WORLD_PTR.get_month(),
    ));
    out.push(format!(
        "player: merchant {local} {} {} of {} | money {} company value {} | traders {traders} ships {ship_count} merchants {merchants}",
        player.get_name(),
        player.get_family_name(),
        get_town_name(player.get_hometown_index()).unwrap_or_else(|| "?".into()),
        player.get_money(),
        player.get_company_value(),
    ));

    // The scan keeps its round counter in the ten-day task's own data at `+0x8`;
    // `counter & 7` is the `captain_index & 7` the next run will process.
    let mut group = None;
    for index in 0..SCHEDULED_TASKS_PTR.get_tasks_size() {
        let task = SCHEDULED_TASKS_PTR.get_scheduled_task(index);
        if task.get_opcode() != SCHEDULED_TASK_OPCODE_TEN_DAY_UPDATE {
            continue;
        }
        let due = task.get_due_timestamp();
        let counter = task.get_data_dword(0x8);
        group = Some(counter & 7);
        out.push(format!(
            "ten-day task {index}: due {due} (in {:.1} days) | data+0x0 {} counter {counter} -> group {}{}",
            due.wrapping_sub(now) as f32 / 256.0,
            task.get_data_dword(0),
            counter & 7,
            if counter > 0x1f { " | SCAN DISABLED, counter past 0x1f" } else { "" },
        ));
    }
    if group.is_none() {
        out.push("ten-day task: NOT FOUND in the queue".to_string());
    }

    // `merchant+0x8 == 0` is a human player (verified in done/pirate-ai.md): his
    // captains take the random path and only his administrators gain at all.
    let human: Vec<bool> = (0..merchants).map(|i| GAME_WORLD_PTR.get_merchant(i).get_control_word() == 0).collect();
    let words: Vec<String> = (0..merchants)
        .map(|i| format!("{i}={:04x}{}", GAME_WORLD_PTR.get_merchant(i).get_control_word(), if human[i as usize] { "*" } else { "" }))
        .collect();
    out.push(format!("merchant control words (* = human, random path): {}", words.join(" ")));

    // Where each record sits: chained to a town = that tavern, `ship+0x42` = that ship,
    // `office+0x2F2` = administrator of that office, anything else unplaced.
    // Which growth path the scan gives this owner's captains. It walks merchants and
    // their ship chains, so a ship with no owner (0xFF - pirate ships and empty slots) is
    // never visited at all.
    let path_of = |owner: u16| {
        if owner >= merchants {
            "no owner, never scanned"
        } else if human[owner as usize] {
            "human 0..50"
        } else {
            "AI +8"
        }
    };
    let mut place: Vec<String> = vec![String::new(); traders as usize];
    for town_index in 0..GAME_WORLD_PTR.get_towns_count() as u8 {
        let town = get_town_name(town_index).unwrap_or_else(|| format!("town {town_index}"));
        let mut index = GAME_WORLD_PTR.get_town(town_index).get_auto_trader_chain_head();
        // The chain ends on an out-of-range index; cap the walk against cycles.
        for _ in 0..traders {
            let Some(trader) = ships.get_auto_trader(index) else { break };
            place[index as usize] = format!("tavern {town}");
            index = trader.get_next_index();
        }
    }
    for ship_index in 0..ship_count {
        let Some(ship) = ships.get_ship(ship_index) else { continue };
        let captain = ship.get_captain_index();
        if captain >= traders {
            continue;
        }
        let owner = ship.get_merchant_index();
        place[captain as usize] = format!(
            "ship {ship_index} {:?} owner {owner:#04x} {}{}",
            ship.get_name(),
            path_of(owner as u16),
            // The one status the scan skips outright.
            if ship.get_status() == 0x11 { " status 0x11 SKIPPED" } else { "" },
        );
    }
    let mut admins: Vec<u16> = Vec::new();
    for office_index in 0..GAME_WORLD_PTR.get_offices_count() {
        let office = GAME_WORLD_PTR.get_office(office_index);
        let admin = office.get_administrator_index();
        if admin >= traders {
            continue;
        }
        admins.push(admin);
        let owner = office.get_merchant_index();
        place[admin as usize] = format!(
            "office {office_index} in {} owner {owner:#04x} {}",
            get_town_name(office.get_town_index()).unwrap_or_else(|| format!("town {}", office.get_town_index())),
            path_of(owner),
        );
    }

    let mut over_cap = 0;
    let mut gated_out = 0;
    let mut lockstep: Vec<String> = Vec::new();
    let mut due_next: Vec<String> = Vec::new();
    for index in 0..traders {
        let Some(trader) = ships.get_auto_trader(index) else { break };
        let (nav, trade, combat) = (trader.get_navigation_skill(), trader.get_trade_skill(), trader.get_combat_skill());
        // A free slot is memset to 0xFF and linked into the freelist through +0x0.
        if trader.get_state_byte() == 0xff && nav == 0xff && trade == 0xff && combat == 0xff {
            continue;
        }
        let (nav_cap, trade_cap, combat_cap) = skill_caps(index);
        let over = |skill: u8, cap: u8| if skill > cap { "!" } else { " " };
        if nav > nav_cap || trade > trade_cap || combat > combat_cap {
            over_cap += 1;
        }
        // Every gain is gated against the NAVIGATION cap, whichever skill was rolled, so
        // a record with all three at or above it never gains again.
        let stuck = nav >= nav_cap && trade >= nav_cap && combat >= nav_cap;
        if stuck {
            gated_out += 1;
        }
        let placed = if place[index as usize].is_empty() {
            "unplaced".to_string()
        } else {
            place[index as usize].clone()
        };
        out.push(format!(
            "trader {index:3} {} nav {nav:3}/{nav_cap}{} trade {trade:3}/{trade_cap}{} combat {combat:3}/{combat_cap}{} | wage {:3} mer {:#04x} state {:#04x} retire {} born {} age {:.1}y | {placed}{}",
            if trader.is_captain() { "CAPT" } else { "PIRA" },
            over(nav, nav_cap),
            over(trade, trade_cap),
            over(combat, combat_cap),
            trader.get_daily_wage(),
            trader.get_merchant_index(),
            trader.get_state_byte(),
            trader.get_retirement_flag(),
            trader.get_timestamp(),
            now.saturating_sub(trader.get_timestamp()) as f32 / TICKS_PER_YEAR as f32,
            if stuck { " | GATED OUT" } else { "" },
        ));
        lockstep.push(format!("{index}:{}", trade as i32 - combat as i32));
        if group == Some(index as u32 & 7) && !place[index as usize].is_empty() {
            due_next.push(index.to_string());
        }
    }

    // The administrator path adds exactly 43 at a time from a fresh 0, so anything else
    // means either a different writer or the record is not really an administrator.
    let stray: Vec<String> = admins
        .iter()
        .filter_map(|&i| ships.get_auto_trader(i).map(|t| (i, t.get_trade_skill())))
        .filter(|(_, trade)| trade % 43 != 0)
        .map(|(i, trade)| format!("{i}={trade}"))
        .collect();
    out.push(format!(
        "administrators: {} | trade not a multiple of 43: {}",
        admins.len(),
        if stray.is_empty() { "none".to_string() } else { stray.join(" ") }
    ));
    out.push(format!("records with a skill above its cap: {over_cap} | gated out of all further gains: {gated_out}"));
    out.push(format!("trade-combat per record (must not move between presses): {}", lockstep.join(" ")));
    out.push(format!(
        "group {} is processed next run, placed records in it: {}",
        group.map(|g| g.to_string()).unwrap_or_else(|| "?".into()),
        if due_next.is_empty() { "none".to_string() } else { due_next.join(" ") }
    ));

    for line in &out {
        debug!("probe1: {line}");
    }
    // Always append, never truncate: the point is to compare presses days or years apart
    // and across saves, so every block from every session stays in the one file.
    let file = std::fs::OpenOptions::new().create(true).append(true).open("_probe1.log");
    if let Ok(mut file) = file {
        use std::io::Write;
        for line in &out {
            let _ = writeln!(file, "{line}");
        }
    }
    notify(&format!(
        "probe1 #{press}: {over_cap} over cap, {gated_out} gated out, {} lines >> _probe1.log",
        out.len()
    ));
}

/// How often F9 has been pressed since the DLL was loaded: press 0 starts a fresh
/// `_probe1.log`, later presses append their own block.
static PROBE1_PRESSES: AtomicU32 = AtomicU32::new(0);

/// ALT+F9 (THROWAWAY): dump every office of the player with its administrator record, to
/// settle whether dismissing and re-hiring an administrator preserves his trade skill.
///
/// Press once with the administrator in place, once after dismissing him, once after
/// hiring again. Appends to `_probe_admin.log`, so the three blocks diff cleanly.
///
/// What the code says should happen: operation `0x5E` frees the record on dismissal
/// (`0x005098B0`) and its hire path unconditionally allocates a fresh one and writes trade
/// `= 0` (`0x0053DA1D`). The two wage formulas tell the record apart from anything the
/// interface computes on its own:
///
/// - an administrator's wage is `0x004FE160`: `20 * (trade / 43) + 10`, so only ever
///   10, 30, 50, 70, 90 or 110;
/// - a captain's is `0x004FE190`: `(nav + trade + combat) / 50 + (state % 11) + 10`.
///
/// So a wage of 14 cannot have come from an administrator record at all, and the offer
/// shown before hiring must be computed somewhere else.
unsafe fn debug_probe_administrators() {
    let press = PROBE_ADMIN_PRESSES.fetch_add(1, Ordering::SeqCst);
    let mut out: Vec<String> = Vec::new();
    let ships = ShipsPtr::new();
    let traders = ships.get_auto_traders_size();
    let merchant_index = OPERATIONS_PTR.get_player_merchant_index();
    out.push(format!(
        "=== admin press {press} | tick {} | player merchant {merchant_index} | traders {traders} ===",
        GAME_WORLD_PTR.get_game_time_raw()
    ));

    // The player's offices, chained from merchant+0xC through office+0x2C8.
    let merchant = GAME_WORLD_PTR.get_merchant(merchant_index as u16);
    let offices = GAME_WORLD_PTR.get_offices_count();
    let mut index = merchant.get_first_office_index();
    for _ in 0..offices {
        if index >= offices {
            break;
        }
        let office = GAME_WORLD_PTR.get_office(index);
        let town = get_town_name(office.get_town_index()).unwrap_or_else(|| format!("town {}", office.get_town_index()));
        let admin = office.get_administrator_index();
        let detail = match ships.get_auto_trader(admin) {
            Some(t) => {
                let (nav, trade, combat) = (t.get_navigation_skill(), t.get_trade_skill(), t.get_combat_skill());
                let admin_wage = 20 * (trade as u32 / 43) + 10;
                let captain_wage = (nav as u32 + trade as u32 + combat as u32) / 50 + (t.get_state_byte() as u32 % 11) + 10;
                format!(
                    "admin {admin}: names {}/{} state {:#04x} nav {nav} trade {trade} (level {}) combat {combat} | wage {} [admin formula {admin_wage}, captain formula {captain_wage}] mer {:#04x} born {}",
                    t.get_first_name_id(),
                    t.get_last_name_id(),
                    t.get_state_byte(),
                    trade / 43,
                    t.get_daily_wage(),
                    t.get_merchant_index(),
                    t.get_timestamp(),
                )
            }
            None => format!("admin index {admin:#06x} - no administrator"),
        };
        out.push(format!("office {index} in {town}: flags {:#04x} | {detail}", office.get::<u8>(0x2d6)));
        index = office.get_next_office_of_merchant_index();
    }

    // Free records are memset to 0xFF and linked into the freelist through +0x0. Watching
    // this list is how a dismissal's free and a hire's re-allocation become visible.
    let free: Vec<String> = (0..traders)
        .filter(|&i| {
            ships
                .get_auto_trader(i)
                .map(|t| t.get_state_byte() == 0xff && t.get_navigation_skill() == 0xff && t.get_trade_skill() == 0xff && t.get_combat_skill() == 0xff)
                .unwrap_or(false)
        })
        .map(|i| i.to_string())
        .collect();
    out.push(format!("free slots ({}): {}", free.len(), free.join(" ")));

    // The whole office record of whatever office is on screen. If anything office-side
    // remembers a dismissed administrator, a diff of this block across the three presses
    // is where it shows up (the ware stock at +0x4 moves on its own, so expect noise).
    let window = UITradingOfficeWindowPtr::new();
    if window.get_address() != 0 {
        let town_index = window.get_town_index() as u8;
        match GAME_WORLD_PTR.get_office_in_of(town_index, merchant_index as _) {
            Some(office) => {
                let town = get_town_name(town_index).unwrap_or_else(|| format!("town {town_index}"));
                out.push(format!("--- office record in {town} at {:#010x}, {OFFICE_SIZE:#x} bytes ---", office.address));
                let mut offset = 0;
                while offset < OFFICE_SIZE {
                    let row: Vec<String> = (0..16.min(OFFICE_SIZE - offset)).map(|i| format!("{:02x}", *((office.address + offset + i) as *const u8))).collect();
                    out.push(format!("{offset:04x}: {}", row.join(" ")));
                    offset += 16;
                }
            }
            None => out.push(format!("no player office in the open window's town ({town_index})")),
        }
    } else {
        out.push("no trading office window open - open one for the hex block".to_string());
    }

    for line in &out {
        debug!("admin: {line}");
    }
    let file = std::fs::OpenOptions::new().create(true).append(true).open("_probe_admin.log");
    if let Ok(mut file) = file {
        use std::io::Write;
        for line in &out {
            let _ = writeln!(file, "{line}");
        }
    }
    notify(&format!("admin probe #{press}: {} lines >> _probe_admin.log", out.len()));
}

/// How often ALT+F9 has been pressed since the DLL was loaded.
static PROBE_ADMIN_PRESSES: AtomicU32 = AtomicU32::new(0);

/// SHIFT+F9 (THROWAWAY): dump the selected ship's whole struct to `_probe1_ship.log`,
/// raw and decoded, one block per press (the file is truncated on the first press of a
/// session). Select a ship, press, change one thing in-game - crew, cutlasses, a gun -
/// press again, and the diff of the two hex blocks names the field that moved.
static PROBE1_SHIP_PRESSES: AtomicU32 = AtomicU32::new(0);

unsafe fn debug_probe1_ship() {
    let ships = ShipsPtr::new();
    let Some(index) = selected_ship_index() else {
        notify("probe1 ship: no ship selected");
        return;
    };
    let Some(ship) = ships.get_ship(index) else {
        notify(&format!("probe1 ship: index {index} out of range"));
        return;
    };
    let press = PROBE1_SHIP_PRESSES.fetch_add(1, Ordering::SeqCst);
    let mut out = Vec::new();
    out.push(format!(
        "=== press {press} tick {} ship {index} \"{}\" type {} upgrade {} ===",
        GAME_WORLD_PTR.get::<u32>(0x14),
        ship.get_name(),
        ship.get::<u8>(0xE),
        ship.get::<u8>(0xF)
    ));
    out.push(format!(
        "crew +0x40 {} (mirror +0x154 {}) | artillery power +0x120 {} weight +0x11C {} | capacity +0x10 {} used +0x118 {} | +0x158 {} | health {}/{}",
        ship.get::<u16>(0x40),
        ship.get::<u16>(0x154),
        ship.get::<i32>(0x120),
        ship.get::<i32>(0x11C),
        ship.get::<i32>(0x10),
        ship.get::<i32>(0x118),
        ship.get::<i32>(0x158),
        ship.get::<i32>(0x18),
        ship.get::<i32>(0x14)
    ));
    // The 12 artillery slots at +0x13C are two bytes each (count, type).
    let slots: Vec<String> = (0..12u32)
        .map(|slot| format!("{:02x}{:02x}", ship.get::<u8>(0x13C + slot * 2), ship.get::<u8>(0x13D + slot * 2)))
        .collect();
    out.push(format!("artillery slots +0x13C: {}", slots.join(" ")));
    // The whole struct, so any field that moves shows up in a diff.
    for row in 0..SHIP_SIZE / 16 {
        let base = row * 16;
        let bytes: Vec<String> = (0..16u32).map(|i| format!("{:02x}", ship.get::<u8>(base + i))).collect();
        out.push(format!("+{base:#05x}  {}", bytes.join(" ")));
    }

    for line in &out {
        debug!("probe1 ship: {line}");
    }
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .append(press > 0)
        .truncate(press == 0)
        .open("_probe1_ship.log");
    if let Ok(mut file) = file {
        use std::io::Write;
        for line in &out {
            let _ = writeln!(file, "{line}");
        }
    }
    notify(&format!(
        "probe1 ship #{press}: {} crew {} arty {} -> _probe1_ship.log",
        ship.get_name(),
        ship.get::<u16>(0x40),
        ship.get::<i32>(0x120)
    ));
}

/// CTRL+F9 (THROWAWAY): the pirate timeline. Samples the pirate convoys into
/// `_probe1_timeline.log` while the game runs, so a multi-day observation needs no key
/// presses: what the restraint counter does across a week, and when a convoy switches
/// target, goes home, enters a battle or vanishes into a hideout.
///
/// The sampling hangs off the ships tick itself (the `call 0x00506720` at `0x00531011`
/// inside `advance_time`), not off a timer, so it sees **every** tick no matter the game
/// speed - including fast-forward, which advances up to a whole day per frame and would
/// let a wall-clock sampler step clean over the transitions. Each tick it computes a
/// cheap fingerprint of the pirate convoys and writes a line only when that changes, or
/// every [TIMELINE_TICKS] ticks as a heartbeat.
static TIMELINE_ON: AtomicBool = AtomicBool::new(false);
/// A quarter of a day between heartbeat samples (a day is 256 ticks).
const TIMELINE_TICKS: u32 = 64;
/// `advance_time`'s call to the ships tick, module-relative for `hook_call_rel32`.
const SHIPS_TICK_CALL_OFFSET: u32 = 0x131011;
static TIMELINE_HOOK_PTR: AtomicPtr<CallRel32Hook> = AtomicPtr::new(std::ptr::null_mut());
static TIMELINE_LOG: Mutex<Option<std::fs::File>> = Mutex::new(None);

unsafe fn toggle_probe1_timeline() {
    let on = !TIMELINE_ON.load(Ordering::SeqCst);
    if on {
        if TIMELINE_HOOK_PTR.load(Ordering::SeqCst).is_null() {
            match hook_call_rel32(SHIPS_TICK_CALL_OFFSET, ships_tick_timeline_hook as usize as u32) {
                Ok(hook) => TIMELINE_HOOK_PTR.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
                Err(e) => {
                    error!("probe1 timeline: hooking the ships tick failed: {e:?}");
                    notify("probe1 timeline: hook failed");
                    return;
                }
            }
        }
        *TIMELINE_LOG.lock().unwrap() = std::fs::File::create("_probe1_timeline.log").ok();
    } else {
        *TIMELINE_LOG.lock().unwrap() = None;
    }
    TIMELINE_ON.store(on, Ordering::SeqCst);
    notify(&format!("probe1 timeline: {}", if on { "on -> _probe1_timeline.log" } else { "off" }));
}

/// Wraps the ships tick: the original first, so the sample shows the state the tick left
/// behind.
#[no_mangle]
unsafe extern "thiscall" fn ships_tick_timeline_hook(this: u32, tick: u32) {
    let orig: extern "thiscall" fn(u32, u32) = mem::transmute((*TIMELINE_HOOK_PTR.load(Ordering::Relaxed)).old_absolute);
    orig(this, tick);
    if TIMELINE_ON.load(Ordering::Relaxed) {
        sample_probe1_timeline(tick);
    }
}

/// The last prey each convoy latched, kept because `0x0050BC40` clears `ship+0x50` when
/// it engages - without this the battle line cannot name who was attacked.
static LAST_PREY: Mutex<[u16; TIMELINE_CONVOYS]> = Mutex::new([0xFFFF; TIMELINE_CONVOYS]);
const TIMELINE_CONVOYS: usize = 64;

unsafe fn sample_probe1_timeline(tick: u32) {
    static LAST_TICK: AtomicU32 = AtomicU32::new(0);
    static LAST_SHAPE: AtomicU32 = AtomicU32::new(0);
    let (shape, changed_at) = (probe1_timeline_shape(), LAST_SHAPE.load(Ordering::Relaxed));
    let due = tick.wrapping_sub(LAST_TICK.load(Ordering::Relaxed)) >= TIMELINE_TICKS;
    let changed = shape != changed_at;
    if !due && !changed {
        return;
    }
    LAST_TICK.store(tick, Ordering::Relaxed);
    LAST_SHAPE.store(shape, Ordering::Relaxed);
    let detail = probe1_timeline_detail();
    use std::io::Write;
    if let Ok(mut guard) = TIMELINE_LOG.lock() {
        if let Some(file) = guard.as_mut() {
            let _ = writeln!(
                file,
                "tick {tick} day {} time {:#04x} {}{detail}",
                tick >> 8,
                tick & 0xFF,
                if changed { "CHANGE " } else { "" }
            );
            let _ = file.flush();
        }
    }
}

/// A cheap fingerprint of every pirate convoy's shape - status, flags, prey and member
/// count. Runs every tick, so it allocates nothing.
unsafe fn probe1_timeline_shape() -> u32 {
    let ships = ShipsPtr::new();
    let ship_count = ships.get_ships_size();
    let convoy_count = ships.get_convoys_size();
    let mut hash: u32 = 0;
    for index in 0..convoy_count {
        let convoy = ships.get_convoy(index).unwrap();
        let status: u16 = convoy.get(0x12);
        if convoy.get::<u8>(0x0) != 0xFF || status == 0xFF {
            continue;
        }
        let acting: u16 = convoy.get(0x10);
        let prey = match ships.get_ship(acting) {
            Some(ship) => ship.get::<u16>(0x50),
            None => 0xFFFF,
        };
        let mut members = 0u32;
        let mut member: u16 = convoy.get(0xA);
        while member < ship_count && members < 8 {
            member = ships.get_ship(member).unwrap().get_next_ship_in_convoy();
            members += 1;
        }
        for value in [index as u32, status as u32, convoy.get::<u16>(0x14) as u32, acting as u32, prey as u32, members] {
            hash = hash.rotate_left(5) ^ value;
        }
    }
    hash
}

/// The line body: every pirate convoy decoded, plus the pirates outside one.
unsafe fn probe1_timeline_detail() -> String {
    let ships = ShipsPtr::new();
    let ship_count = ships.get_ships_size();
    let convoy_count = ships.get_convoys_size();
    let mut detail = String::new();
    for index in 0..convoy_count {
        let convoy = ships.get_convoy(index).unwrap();
        let status: u16 = convoy.get(0x12);
        if convoy.get::<u8>(0x0) != 0xFF || status == 0xFF {
            continue;
        }
        let acting: u16 = convoy.get(0x10);
        let flags: u16 = convoy.get(0x14);
        let counter: u16 = convoy.get(0x16);
        let mut members = Vec::new();
        let mut member: u16 = convoy.get(0xA);
        let mut hops = 0;
        while member < ship_count && hops < 8 {
            members.push(member.to_string());
            member = ships.get_ship(member).unwrap().get_next_ship_in_convoy();
            hops += 1;
        }
        let (position, prey_text) = match ships.get_ship(acting) {
            Some(ship) => {
                let prey: u16 = ship.get(0x50);
                let prey_text = match ships.get_ship(prey) {
                    Some(prey_ship) => format!(
                        "{prey}:{} m{:#04x} at {},{}",
                        prey_ship.get_name(),
                        prey_ship.get_merchant_index(),
                        prey_ship.get::<u16>(0x1E),
                        prey_ship.get::<u16>(0x22)
                    ),
                    None => "none".to_string(),
                };
                (format!("{},{}", ship.get::<u16>(0x1E), ship.get::<u16>(0x22)), prey_text)
            }
            None => ("?".to_string(), "?".to_string()),
        };
        // Remember the prey while it is still there, and name it once a battle starts.
        let victim = {
            let mut last = LAST_PREY.lock().unwrap();
            let slot = (index as usize).min(TIMELINE_CONVOYS - 1);
            let prey: u16 = match ships.get_ship(acting) {
                Some(ship) => ship.get(0x50),
                None => 0xFFFF,
            };
            if prey != 0xFFFF {
                last[slot] = prey;
            }
            match ships.get_ship(last[slot]) {
                Some(ship) => format!(
                    "{}:{} m{:#04x}{}",
                    last[slot],
                    ship.get_name(),
                    ship.get_merchant_index(),
                    if ship.get_merchant_index() as u32 == *(0x006DFC14 as *const u32) { " MINE" } else { "" }
                ),
                None => "unknown".to_string(),
            }
        };
        detail.push_str(&format!(
            "| convoy {index} status {status:#x} flags {flags:#06x} counter {counter} ({:.2}d) acting {acting} at {position} ships {} prey {prey_text} {}",
            counter as f64 / 256.0,
            members.join(","),
            if status == 0x14 { format!("ENGAGED victim {victim} ") } else { String::new() }
        ));
    }
    // Pirate ships outside a convoy: at a hideout, or waiting to be dispatched.
    let mut loose = Vec::new();
    for index in 0..ship_count {
        let ship = ships.get_ship(index).unwrap();
        if ship.get_status() != 0x12 || ship.get_convoy_id() < convoy_count {
            continue;
        }
        loose.push(format!(
            "{index}@{},{} hp{}",
            ship.get::<u16>(0x1E),
            ship.get::<u16>(0x22),
            ship.get::<i32>(0x18)
        ));
    }
    detail.push_str(&format!("| loose {}", loose.join(" ")));
    detail
}

/// SHIFT+F10: the ice census - every town's cold accumulator, ice level and frozen
/// flag, plus the pending thaw tasks. Written to validate the port-freeze model in
/// `.claude/notes/done/port-freezing.md`, which was derived statically:
///
/// - the daily ice pass (`0x004E45C4`, scheduled task `0x0D`) runs only when the day
///   of the year is `<= 58` or `>= 333`;
/// - it grows `town+0x9B8` in winter and melts it otherwise, using a per-climate term
///   from the 52-byte-stride table at `0x006DDBB0`, indexed by `[0x006DE4B8 + idx]`;
/// - `level = (cold >> 9) + 2` goes to `town+0x9BD`, and bit `0x80` is set when the
///   level exceeds 5 - that bit alone is what makes a port freezable;
/// - an eligible, not-yet-frozen port then takes a `rand(0x6400) < 0x180` roll (1.5%)
///   each day, and on a hit sets [TOWN_FLAG_FROZEN] and schedules opcode `0x35` for
///   `(level & 0x7F) + 1` days later.
///
/// What to look for, pressing it on a few consecutive days over a winter:
///
/// 1. **`cold` moves the right way**: rising while the day of the year is outside
///    `58..333`, falling (or pinned at 0) inside it. `dLevel` names the change since
///    the previous press so a rise is visible without diffing by eye.
/// 2. **the stored level byte equals the modelled one** - `lvl(raw/exp)` prints both,
///    and `MISMATCH` marks a disagreement. Because the model covers the whole byte,
///    that one check validates the level arithmetic and the `0x80` eligibility bit at
///    once. A mismatch means the model is wrong, not the town. Verified clean on
///    27 Aug 2026 across all 24 towns of a 1301 world, on a press taken while the pass
///    was running.
/// 3. **`ELIGIBLE` appears exactly when `level > 5`** (`cold >= 0x800`; the `to2k`
///    column is the cold still missing), and only eligible towns ever gain `FROZEN`. A
///    frozen town that never showed `ELIGIBLE` would break the derivation.
/// 4. **Every `FROZEN` town has a matching pending `0x35` task**, and its due date is
///    within `level + 1` days of when it froze. The task list at the bottom prints
///    each one with its town and the days remaining; `FROZEN, NO THAW TASK` flags a
///    frozen port with nothing scheduled to reopen it - that would be a stuck port.
/// 5. **The climate term differs by region**: grouping the `clim`/`term` columns
///    against which towns actually freeze is what would let us say how often each
///    town freezes. That is the open question the static trace could not answer.
///
/// Appended to `_probe_ice.log`, never truncated, so presses days apart compare
/// directly.
unsafe fn debug_probe_ice() {
    let mut out: Vec<String> = Vec::new();
    let towns = GAME_WORLD_PTR.get_towns_count();
    let day_of_year = GAME_WORLD_PTR.get_day_of_year();
    let now = GAME_WORLD_PTR.get_game_time_raw();
    let press = ICE_PRESSES.fetch_add(1, Ordering::SeqCst);
    // The ice pass's own season gate, from its caller 0x004E4984.
    let ice_season = day_of_year <= 58 || day_of_year >= 333;
    // Which arm of the pass today takes (0x004E46AE), which decides what the numbers
    // below even mean. Measured 27 Aug 2026: the pass runs TWICE a day, so the per-pass
    // rates double.
    let phase = if !ice_season {
        "IDLE, pass does not run (day 59..332)"
    } else if day_of_year >= 333 || day_of_year < 32 {
        "ACCUMULATING, cold += (rand(60) + edi) / 3 per pass, 2 passes/day"
    } else if day_of_year <= 57 {
        "DECAYING, cold -= (rand(edi) + 2.5 * edi) / 3 per pass, mean edi, 2 passes/day"
    } else {
        "RESET day 58, cold zeroed and level forced to 1"
    };

    out.push(format!(
        "=== ice press {press} | tick {now} year {} day-of-year {day_of_year} ({}.{}) | {phase} ===",
        GAME_WORLD_PTR.get_year(),
        GAME_WORLD_PTR.get_day_of_month(),
        GAME_WORLD_PTR.get_month(),
    ));
    out.push(
        "town                 cold to2k lvl(raw/exp) prev clim  edi term       0x9C0 0x9C4 state"
            .to_string(),
    );

    let mut frozen: Vec<u8> = Vec::new();
    let mut eligible = 0usize;
    let mut previous = ICE_PREVIOUS.lock();

    for index in 0..towns.min(40) as u8 {
        let town = GAME_WORLD_PTR.get_town(index);
        let name = get_town_name(index).unwrap_or_else(|| format!("town {index}"));
        let cold = town.get_cold_accumulator();
        let level_byte = town.get_ice_level();
        let level = level_byte & 0x7f;
        // The pass's own arithmetic, byte for byte (`0x004E47DF`): `al = (cold >> 9) + 2`
        // in BYTE registers, then `al |= 0x80` when that exceeds 5. Modelling the whole
        // byte makes this one comparison validate the level and the eligibility bit
        // together, and it keeps working past `(cold >> 9) + 2 >= 0x80`, where the level
        // aliases into the eligibility bit.
        let expected = {
            let base = (((cold >> 9) & 0xff) as u8).wrapping_add(2);
            if base > 5 {
                base | 0x80
            } else {
                base
            }
        };
        // The one exception (`0x004E47CD`): with cold at 0 the pass writes level 1 and
        // stops - but only on a pass that runs outside its own winter window, which is
        // day 58 alone, the single day the caller's gate (`day <= 58`) and the pass's own
        // test (`day >= 58` is not winter) disagree. That 1 then sits there untouched all
        // summer, so an off-season press legitimately reads level 1 against a computed 2.
        // Measured 27 Aug 2026: every town, every off-season press.
        let matches_model = level_byte == expected || (cold == 0 && level_byte == 1);
        // Cold still needed before the port can freeze at all: eligibility is level > 5,
        // i.e. cold >= 0x800.
        let to_eligible = 0x800u32.saturating_sub(cold);
        let previous_level = town.get::<u8>(0x9bc);
        let climate = *((0x006de4b8 + index as u32) as *const u8);
        // Per-climate block at 0x006DDBB0, stride 52 (0x34); the pass reads the first
        // two dwords of the block as its accumulate/melt terms.
        let block = 0x006ddbb0 + climate as u32 * 52;
        let term = *(block as *const u32);
        let term2 = *((block + 4) as *const u32);
        // The single number the pass derives from the block (0x004E468C-0x004E46AB) and
        // uses for both arms. Decay is mean `edi` per pass, measured to 2.004 passes/day
        // across all 24 towns of a 1363 world.
        let edi = (term as i64 - 2 * term2 as i64 + 3600) / 100;
        let is_frozen = town.is_port_frozen();
        if is_frozen {
            frozen.push(index);
        }
        let freezable = level_byte & 0x80 != 0;
        if freezable {
            eligible += 1;
        }

        let mut state = String::new();
        if is_frozen {
            state.push_str("FROZEN ");
        }
        if freezable {
            state.push_str("ELIGIBLE ");
        }
        if !matches_model {
            state.push_str(&format!("MISMATCH exp {expected:#04x} "));
        }
        // Movement since the last press, which is the whole point of pressing twice.
        if let Ok(previous) = previous.as_mut() {
            if let Some((old_cold, old_level)) = previous[index as usize] {
                let delta = cold as i64 - old_cold as i64;
                if delta != 0 || old_level != level_byte {
                    state.push_str(&format!("dCold {delta:+} dLevel {}->{} ", old_level & 0x7f, level));
                }
            }
            previous[index as usize] = Some((cold, level_byte));
        }

        out.push(format!(
            "{name:<18} {cold:>6} {to_eligible:>4} {level:>3}({level_byte:#04x}/{expected:#04x}) {previous_level:>4} {climate:>4} {edi:>4} {term:>5}/{term2:<5} {:>5} {:#04x}  {state}",
            town.get::<u32>(0x9c0),
            town.get::<u8>(0x9c4),
        ));
    }

    // Pending thaws: the ice pass schedules one per freeze, so a frozen port with no
    // task is a stuck port and a task with no frozen port is a stale schedule.
    let mut thaw_towns: Vec<u8> = Vec::new();
    out.push("--- pending thaw tasks (opcode 0x35) ---".to_string());
    for index in 0..SCHEDULED_TASKS_PTR.get_tasks_size() {
        let task = SCHEDULED_TASKS_PTR.get_scheduled_task(index);
        if task.get_opcode() != SCHEDULED_TASK_OPCODE_UNFREEZE_PORT {
            continue;
        }
        // data+0, i.e. task+0x8 - and `get_data_dword` already adds the 0x8 data-union
        // base, so this takes offset 0. Passing 0x8 here read task+0x10, a stale dword
        // of the union, which is what produced bogus STALE / NO THAW TASK pairs on
        // 27 Aug 2026.
        let town_index = task.get_data_dword(0) as u8;
        thaw_towns.push(town_index);
        let due = task.get_due_timestamp();
        out.push(format!(
            "  task {index}: town {town_index} ({}) due {due} = {} days from now{}",
            get_town_name(town_index).unwrap_or_else(|| "?".into()),
            (due.saturating_sub(now)) as f32 / 256.0,
            if GAME_WORLD_PTR.get_town(town_index).is_port_frozen() { "" } else { "  STALE, town not frozen" },
        ));
    }
    for index in &frozen {
        if !thaw_towns.contains(index) {
            out.push(format!(
                "  town {index} ({}) FROZEN, NO THAW TASK",
                get_town_name(*index).unwrap_or_else(|| "?".into())
            ));
        }
    }

    let summary = format!(
        "ice #{press}: {} frozen, {eligible} eligible, {} thaw tasks, day {day_of_year} ({})",
        frozen.len(),
        thaw_towns.len(),
        phase.split(',').next().unwrap_or(phase),
    );
    out.push(summary.clone());

    for line in &out {
        debug!("ice: {line}");
    }
    let file = std::fs::OpenOptions::new().create(true).append(true).open("_probe_ice.log");
    if let Ok(mut file) = file {
        use std::io::Write;
        for line in &out {
            let _ = writeln!(file, "{line}");
        }
    }
    notify(&format!("{summary} >> _probe_ice.log"));
}

/// How often the ice census has been pressed since load.
static ICE_PRESSES: AtomicU32 = AtomicU32::new(0);
/// Last press's (cold, level byte) per town, so each press can report the movement
/// since the previous one - the readable form of "press it on two consecutive days".
/// Indexed by town index; 40 is the world's town capacity (`game_world+0x18`).
static ICE_PREVIOUS: Mutex<[Option<(u32, u8)>; 40]> = Mutex::new([None; 40]);

// ---------------------------------------------------------------------------
// The captain-retirement hang (scheduled task 0x27). See
// `.claude/notes/todo/captain-retire-hang.md`.
//
// The handler `0x004DDC00` re-finds the ship when the (ship, captain) pair it was
// scheduled with no longer matches, and that search contains a two-instruction
// infinite loop at `0x004DDCB0`/`0x004DDCB2` (`cmp ecx,ebp` / `jne -4`, neither
// operand written in between). The window in which a real game can produce a
// mismatch is minutes wide and not player-controllable, so the only way to see it
// is to schedule the task by hand with a deliberately mismatched pair.
//
// Set-up: a save with two ships named `test1` (has a captain) and `test2` (no
// captain). SAVE THE GAME FIRST - the mismatched run is expected to freeze with no
// crash report, because nothing faults.
//
// - ALT+F10        inspect: reads everything, writes nothing, and PREDICTS whether
//                  the mismatched run will hang. Always run this first.
// - CTRL+ALT+F10   benign: schedules the task with the MATCHING pair
//                  (test1, test1's captain). Should quietly take the captain off
//                  test1 and not freeze - which is what proves the probe itself
//                  works before the real test.
// - SHIFT+ALT+F10  the test: schedules the MISMATCHED pair (test2, test1's
//                  captain). Expected to freeze.
// ---------------------------------------------------------------------------

/// `thiscall(tasks, due) -> record*`, `ret 4`. Pops the scheduled-task freelist
/// (growing the array by 0x80 records when it is empty), links the record at the head
/// of the task list, stores `due` at `record+0x0` and re-sorts. The caller then fills
/// in the opcode at `+0x6` and the data union from `+0x8` - exactly what the ice pass
/// at `0x004E48D7` does, which is the known-good caller this mirrors.
const SCHEDULE_TASK: u32 = 0x004D8CF0;
const SCHEDULED_TASKS_ADDRESS: u32 = 0x006DD73C;
/// The scheduling clock, in 1/256-day units - the same base the ice pass adds its
/// delay to. Due = now means the next dispatcher pass picks the task up.
const SCHEDULE_CLOCK_ADDRESS: *const u32 = 0x006DE4B4 as _;
const RETIRE_TASK_OPCODE: u16 = 0x27;
const RETIRE_PROBE_LOG: &str = "_captain_retire_probe.log";

/// Append one line and flush it to disk. `sync_all` matters here: the mismatched run
/// is expected to hang, and the log is the only record of what was scheduled - there
/// is no crash report, because nothing faults.
fn retire_log(line: &str) {
    debug!("retire-probe: {line}");
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(RETIRE_PROBE_LOG) {
        use std::io::Write;
        let _ = writeln!(file, "{line}");
        let _ = file.sync_all();
    }
}

/// The two set-up ships and the captain to schedule, or `None` with the reason logged.
unsafe fn retire_probe_ships() -> Option<(u16, u16, u16)> {
    let ships = p3_api::ships::ShipsPtr::new();
    let Some((with_captain, with_index)) = ships.get_ship_by_name("test1") else {
        retire_log("ABORT: no ship named \"test1\" - the probe needs the prepared save");
        return None;
    };
    let Some((_, without_index)) = ships.get_ship_by_name("test2") else {
        retire_log("ABORT: no ship named \"test2\" - the probe needs the prepared save");
        return None;
    };
    let captain = with_captain.get_captain_index();
    if captain as u32 >= ships.get_auto_traders_size() as u32 {
        retire_log(&format!(
            "ABORT: test1 (ship {with_index}) has no captain (field_42 = {captain:#06x}); the probe needs one"
        ));
        return None;
    }
    Some((with_index, without_index, captain))
}

/// Walk the handler's own search the way it walks it, and report where it would land.
/// The merchant loop counts DOWN from the merchant count, and the first merchant whose
/// first-ship index is in range is the one whose ship gets compared - so whether the
/// hang fires is decidable before running it.
unsafe fn retire_probe_predict(captain: u16) {
    let ships = p3_api::ships::ShipsPtr::new();
    let ship_count = ships.get_ships_size();
    let merchant_count = GAME_WORLD_PTR.get_merchants_count();

    for counter in (1..=merchant_count).rev() {
        let merchant_index = counter - 1;
        let first_ship = GAME_WORLD_PTR.get_merchant(merchant_index).get_first_ship_index();
        if first_ship >= ship_count {
            continue;
        }
        // 0x004DDC97 takes this merchant, and 0x004DDCAB compares this ship's captain.
        let its_captain = ships.get_ship(first_ship).map(|s| s.get_captain_index()).unwrap_or(0xffff);
        let name = ships.get_ship(first_ship).map(|s| s.get_name()).unwrap_or_default();
        retire_log(&format!(
            "  search lands on merchant {merchant_index} (of {merchant_count}), its first ship {first_ship} {name:?} carrying captain {its_captain}"
        ));
        if its_captain == captain {
            retire_log(
                "  PREDICTION: NO hang - that first ship already carries the wanted captain, so the jne is not taken. Move test1 out of that merchant's chain head to get a real test.",
            );
        } else {
            retire_log("  PREDICTION: HANG - the compare fails and 0x004DDCB2 loops on itself forever");
        }
        return;
    }
    retire_log("  PREDICTION: no hang - no merchant has a first ship in range, so the search bails out at 0x004DDCC5");
}

/// ALT+F10: read-only. Reports the set-up and predicts the mismatched run's outcome.
unsafe fn retire_probe_inspect() {
    let ships = p3_api::ships::ShipsPtr::new();
    retire_log(&format!(
        "=== inspect | tick {} year {} day {} ===",
        GAME_WORLD_PTR.get_game_time_raw(),
        GAME_WORLD_PTR.get_year(),
        GAME_WORLD_PTR.get_day_of_year(),
    ));
    let Some((with_index, without_index, captain)) = retire_probe_ships() else {
        notify("retire probe: set-up missing, see _captain_retire_probe.log");
        return;
    };
    let without_captain = ships.get_ship(without_index).map(|s| s.get_captain_index()).unwrap_or(0xffff);
    retire_log(&format!(
        "  test1 = ship {with_index}, captain {captain} | test2 = ship {without_index}, field_42 = {without_captain:#06x}"
    ));
    retire_log(&format!(
        "  benign run would schedule (ship {with_index}, captain {captain}) - pair MATCHES, no search"
    ));
    retire_log(&format!(
        "  test run would schedule (ship {without_index}, captain {captain}) - pair MISMATCHES, enters the search"
    ));
    retire_probe_predict(captain);
    retire_probe_pending();
    notify(&format!("retire probe: test1=ship {with_index} cap {captain}, test2=ship {without_index} >> log"));
}

/// Any task 0x27 still in the queue. After a run that did not freeze, "none pending" is
/// what shows the handler actually ran to completion rather than the task being dropped.
unsafe fn retire_probe_pending() {
    let now = *SCHEDULE_CLOCK_ADDRESS;
    let mut found = 0;
    for index in 0..SCHEDULED_TASKS_PTR.get_tasks_size() {
        let task = SCHEDULED_TASKS_PTR.get_scheduled_task(index);
        if task.get_opcode() != RETIRE_TASK_OPCODE {
            continue;
        }
        found += 1;
        retire_log(&format!(
            "  pending task {index}: opcode 0x27, ship {}, captain {}, due {} ({} ticks from now)",
            task.get_data_dword(0),
            task.get_data_dword(4),
            task.get_due_timestamp(),
            task.get_due_timestamp() as i64 - now as i64,
        ));
    }
    if found == 0 {
        retire_log("  no task 0x27 pending - anything scheduled earlier has been consumed");
    }
}

/// Schedule task 0x27 due now, with the given data union. Mirrors `0x004E48D7`.
unsafe fn retire_probe_schedule(ship_index: u16, captain: u16) {
    let due = *SCHEDULE_CLOCK_ADDRESS;
    let allocate: extern "thiscall" fn(tasks: u32, due: u32) -> u32 = mem::transmute(SCHEDULE_TASK);
    let record = allocate(SCHEDULED_TASKS_ADDRESS, due);
    if record == 0 {
        retire_log("  ABORT: the task allocator returned null");
        return;
    }
    // +0x4 is the list link the allocator just set - do not touch it.
    *((record + 0x6) as *mut u16) = RETIRE_TASK_OPCODE;
    *((record + 0x8) as *mut u32) = ship_index as u32;
    *((record + 0xc) as *mut u32) = captain as u32;
    retire_log(&format!(
        "  scheduled task {RETIRE_TASK_OPCODE:#04x} at record {record:#010x} due {due} with ship {ship_index} captain {captain} - handing control back to the game NOW"
    ));
}

/// CTRL+ALT+F10: the matching pair. Validates the probe - should not freeze.
unsafe fn retire_probe_benign() {
    retire_log(&format!("=== BENIGN run (matching pair) | tick {} ===", GAME_WORLD_PTR.get_game_time_raw()));
    let Some((with_index, _, captain)) = retire_probe_ships() else {
        notify("retire probe: set-up missing, see _captain_retire_probe.log");
        return;
    };
    notify("retire probe: benign run, expect test1 to lose its captain");
    retire_probe_schedule(with_index, captain);
}

/// SHIFT+ALT+F10: the mismatched pair. EXPECTED TO FREEZE THE GAME.
unsafe fn retire_probe_hang() {
    retire_log(&format!(
        "=== HANG TEST (mismatched pair) | tick {} === EXPECTED TO FREEZE - if the log ends here, 0x004DDCB2 span forever",
        GAME_WORLD_PTR.get_game_time_raw()
    ));
    let Some((_, without_index, captain)) = retire_probe_ships() else {
        notify("retire probe: set-up missing, see _captain_retire_probe.log");
        return;
    };
    retire_probe_predict(captain);
    notify("retire probe: HANG TEST armed - the game is expected to freeze now");
    retire_probe_schedule(without_index, captain);
    // NOT a survival test: scheduling returns immediately and the handler runs on a
    // later dispatcher pass, so reaching this line proves nothing. (An earlier version
    // logged "SURVIVED" here, which was simply wrong - it printed moments before the
    // game froze.) The outcome is observable in the game instead: a freeze is the bug;
    // if the game keeps running, press ALT+F10 and check that no 0x27 task is left
    // pending, which means the handler completed.
    retire_log("  task is queued; the handler runs on the next dispatcher pass. Frozen now = the bug. Still running = press ALT+F10 to check the task was consumed.");
}

// ---------------------------------------------------------------------------
// The operation logger. Moved here from mod-auto-supply on 28 Aug 2026: it is a
// debugging tool rather than a gameplay feature, and it had been sitting there
// uncalled since the F9/F10 keys moved into this crate.
//
// It names the opcode behind any UI action in seconds, which is why it is the one
// probe worth keeping permanently. Write-up: `.claude/notes/tools/operation-queue.md`.
// ---------------------------------------------------------------------------

/// Module-relative offset of the operation queue's drain call into the operation switch
/// (`execute_operations` `0x00546870` calls `0x00535760` at `0x00546934`).
const OP_SWITCH_DRAIN_CALL_OFFSET: u32 = 0x146934;
/// Noisy periodic opcodes to omit, or the log drowns in them.
const OP_LOGGER_NOISE: [u32; 3] = [0x94, 0x24, 0x7b];

static OP_LOGGER_HOOK: AtomicPtr<CallRel32Hook> = AtomicPtr::new(std::ptr::null_mut());

unsafe extern "thiscall" fn op_logger_hook(op: u32) {
    let opcode = *(op as *const u32);
    if !OP_LOGGER_NOISE.contains(&opcode) {
        let bytes: Vec<String> = (0..0x14).map(|i| format!("{:02x}", *((op + i) as *const u8))).collect();
        debug!("op {opcode:#04x}: {}", bytes.join(" "));
    }
    let hook = OP_LOGGER_HOOK.load(Ordering::SeqCst);
    let original: extern "thiscall" fn(u32) = mem::transmute((*hook).old_absolute);
    original(op);
}

/// CTRL+SHIFT+F9: start logging every operation the queue drains, so the next UI action
/// names its own opcode. Press once, do the thing in-game, read DebugView.
///
/// Installing is one-way for the session - the hook stays until the game exits - and
/// pressing again is a no-op rather than a second hook.
unsafe fn install_op_logger() {
    if !OP_LOGGER_HOOK.load(Ordering::SeqCst).is_null() {
        notify("op logger: already running");
        return;
    }
    match hook_call_rel32(OP_SWITCH_DRAIN_CALL_OFFSET, op_logger_hook as usize as u32) {
        Ok(hook) => {
            OP_LOGGER_HOOK.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst);
            notify("op logger: running - perform the action to identify it");
        }
        Err(e) => error!("op logger: hook failed: {e:?}"),
    }
}
