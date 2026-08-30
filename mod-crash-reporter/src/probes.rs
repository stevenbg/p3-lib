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
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use std::mem;

use hooklet::windows::x86::{hook_call_rel32, CallRel32Hook};
use log::{debug, error, info};
use num_traits::FromPrimitive;
use p3_api::{
    data::{convoy::CONVOY_SIZE, enums::WareId, office::OFFICE_SIZE, p3_ptr::P3Pointer},
    game_world::{GAME_WORLD_PTR, },
    hotkeys::{HotkeysApi, MOD_ALT, MOD_CTRL, MOD_SHIFT},
    operations::OPERATIONS_PTR,
    scheduled_tasks::{
        scheduled_task::{SCHEDULED_TASK_OPCODE_UNFREEZE_PORT},
        SCHEDULED_TASKS_PTR,
    },
    ship::SHIP_SIZE,
    ships::ShipsPtr,
    town::get_town_name,
    ui::{ui_ship_panel::UIShipPanelPtr, ui_trading_office_window::UITradingOfficeWindowPtr},
};
use windows::core::s;
use windows::Win32::System::LibraryLoader::GetModuleHandleA;
use windows::Win32::System::Diagnostics::Debug::{AddVectoredExceptionHandler, GetThreadContext, SetThreadContext, CONTEXT, EXCEPTION_POINTERS};
use windows::Win32::System::Memory::{VirtualQuery, MEMORY_BASIC_INFORMATION, MEM_COMMIT, PAGE_GUARD, PAGE_NOACCESS};
use windows::Win32::System::Threading::GetCurrentThread;
use windows::Win32::UI::Input::KeyboardAndMouse::{VK_F10, VK_F9};

/// The two throwaway probe keys. Neither name says anything beyond "probe slot", and
/// deliberately so: the bodies behind them are rewritten per investigation, so naming a key
/// after whatever it happens to do this week only ages into a lie. Name the handlers, not
/// the keys.
const DEBUG_PROBE1_KEY: u32 = VK_F9.0 as u32;
const DEBUG_PROBE2_KEY: u32 = VK_F10.0 as u32;

/// The registry binding, null when hotkeys.dll is unavailable (probes inert).
static HOTKEYS: AtomicPtr<HotkeysApi> = AtomicPtr::new(std::ptr::null_mut());
const OWNER: &std::ffi::CStr = c"crash-reporter debug probes";

const PROBE_KEYS: [(u32, u32); 14] = [
    (DEBUG_PROBE1_KEY, 0),
    (DEBUG_PROBE1_KEY, MOD_CTRL),
    (DEBUG_PROBE1_KEY, MOD_SHIFT),
    (DEBUG_PROBE1_KEY, MOD_ALT),
    // The operation logger: the one permanent tool here, rather than a throwaway.
    (DEBUG_PROBE1_KEY, MOD_CTRL | MOD_SHIFT),
    // The d3d9 resource-list detector: one-shot check, and the continuous watch.
    (DEBUG_PROBE1_KEY, MOD_CTRL | MOD_ALT),
    (DEBUG_PROBE1_KEY, MOD_SHIFT | MOD_ALT),
    (DEBUG_PROBE2_KEY, 0),
    (DEBUG_PROBE2_KEY, MOD_CTRL),
    (DEBUG_PROBE2_KEY, MOD_SHIFT),
    // The captain-retirement hang probe. Exact-match modifiers, so these three never
    // collide with each other or with plain F10.
    (DEBUG_PROBE2_KEY, MOD_ALT),
    (DEBUG_PROBE2_KEY, MOD_ALT | MOD_CTRL),
    (DEBUG_PROBE2_KEY, MOD_ALT | MOD_SHIFT),
    // The convoy-leader probe, for teaching mod-auto-supply's route keys to work on a
    // convoy selection.
    (DEBUG_PROBE2_KEY, MOD_CTRL | MOD_SHIFT),
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
        (DEBUG_PROBE1_KEY, m) if m == MOD_CTRL | MOD_ALT => debug_probe_d3d9_list(),
        (DEBUG_PROBE1_KEY, m) if m == MOD_SHIFT | MOD_ALT => toggle_probe_d3d9_watch(),
        (DEBUG_PROBE1_KEY, 0) => debug_probe_route_travel_time(),
        (DEBUG_PROBE2_KEY, MOD_CTRL) => debug_probe_dialog_modes(),
        (DEBUG_PROBE2_KEY, MOD_SHIFT) => debug_probe_ice(),
        (DEBUG_PROBE2_KEY, MOD_ALT) => retire_probe_inspect(),
        (DEBUG_PROBE2_KEY, m) if m == MOD_ALT | MOD_CTRL => retire_probe_benign(),
        (DEBUG_PROBE2_KEY, m) if m == MOD_ALT | MOD_SHIFT => retire_probe_hang(),
        (DEBUG_PROBE2_KEY, m) if m == MOD_CTRL | MOD_SHIFT => debug_probe_convoy(),
        (DEBUG_PROBE2_KEY, 0) => debug_probe_town_levy(),
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
/// F10 (rewritten per investigation, 30 Aug 2026): is the low byte of `town+0x6F8` the
/// TAX level? The satisfaction town-modifier block subtracts `low_byte^2 / 20`
/// (`0x0051CA7D`); the byte defaults to 10 at town init (`0x00528C64`) and the setter
/// `0x00529FF2` writes a new low byte, ORs the date serial into the upper bits, and puts
/// an amount into `town+0x6F4`.
///
/// Protocol: open the log, press F10 (baseline, all towns), change one town's tax, press
/// again. If the low byte follows the tax slider and the upper bits jump to "now", the
/// field is the tax level and the satisfaction penalty is tax^2/20.
unsafe fn debug_probe_town_levy() {
    let now = GAME_WORLD_PTR.get_game_time_raw();
    let mut out: Vec<String> = vec![format!("=== town levy dump | tick {now} ({now:#010x}) ===")];
    for i in 0..GAME_WORLD_PTR.get_towns_count() as u8 {
        let town = GAME_WORLD_PTR.get_town(i);
        if town.address == 0 {
            continue;
        }
        let f4 = *((town.address + 0x6f4) as *const i32);
        let f8 = *((town.address + 0x6f8) as *const u32);
        let level = f8 & 0xff;
        let stamp = f8 & !0xff;
        let sat = town.get_satisfactions();
        out.push(format!(
            "{:12} +0x6F4 {f4:8} | +0x6F8 level {level:3} stamp {stamp:#010x} ({}) | sat r/w/p {}/{}/{}",
            get_town_name(i).unwrap_or_else(|| format!("<{i}>")),
            if stamp == 0 { "never".into() } else { format!("{:.0} days ago", (now.wrapping_sub(stamp)) as f64 / 256.0) },
            sat[0], sat[1], sat[2],
        ));
    }
    for l in &out {
        info!("{l}");
    }
    append_probe_log("_probe_tax.log", &out);
    notify("town levy dumped");
}

#[allow(dead_code)]
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
/// F9 (rewritten per investigation, 30 Aug 2026): dump every scheduled-task record of
/// kind 0x2C - the quarterly wealth-growth governor (`0x004DEA20`, see
/// `todo/game-settings.md`). Two open questions this dump answers across two presses a
/// game-quarter apart:
///
/// - is the record's `+0x8` the previous WEALTH (the handler stores it at `0x004DEB4A`)
///   or a DATE serial (the dispatcher stub restamps "data+0x8" at `0x004D8937`)? If both
///   writes hit the same field the governor is broken vanilla. The dump prints the game
///   time and the richest player's wealth beside it, so the value identifies itself.
/// - how does `+0xC` (the governed rate) move once the wealth ratio bands?
///
/// The task collection is the object at `0x006DD73C`: `+0x0` the record array (stride
/// 0x18), the record count word at `0x006DD748` (valid slots are `< count`, the same test
/// the auction reader uses). Record: `+0x0` due time dword, `+0x4` word, `+0x6` kind
/// word, `+0x8`..`+0x17` four data dwords.
/// F9: travel-time breakdown of the SELECTED ship's trade route.
///
/// Per leg (consecutive route towns, closing back to the first): the router's distance
/// and the game's own travel-time formula (`0x00516A2E`):
/// `8 * distance / ((base_speed * capacity_factor >> 12) * health_factor >> 10)`,
/// assuming **full load and 100% hull** - capacity_factor = 4096 - 614 (the caller at
/// `0x00516A2E` computes `4096 - 614 * cargo_raw / capacity_raw`), health_factor = 256
/// (`clamp(165 + 130 * health / max_health, 204, 256)`).
///
/// The output unit is not pinned yet, so both the raw figure and raw/256 (the
/// 1/256-day candidate) are printed - compare against an in-game ETA to calibrate.
/// The idle line assumes the 6-hour dwell per stop (the in-port counter `ship+0x138`
/// is acted on at 0x40 = 64 ticks = 6 h), so a measured lap can confirm both at once.
unsafe fn debug_probe_route_travel_time() {
    const FULL_LOAD_CAPACITY_FACTOR: u32 = 4096 - 614;
    const FULL_HEALTH_FACTOR: u32 = 256;
    const IDLE_TICKS_PER_STOP: u32 = 0x40;

    let Some(selected) = selected_ship_index() else {
        notify("route time probe: no ship selected");
        return;
    };
    let ships = ShipsPtr::new();
    let Some(ship) = ships.get_ship(selected) else {
        notify("route time probe: selection does not resolve");
        return;
    };
    let ship_type = ship.get_type();

    // The route's towns in chain order (the sum over a closed loop is rotation-proof).
    let pool = *ROUTE_STOP_POOL;
    let pool_count = *ROUTE_STOP_POOL_COUNT;
    let head = *((ship.address + SHIP_ROUTE_HEAD_OFFSET) as *const u16);
    let mut towns: Vec<u8> = Vec::new();
    if pool != 0 && head < pool_count {
        let mut idx = head;
        for _ in 0..64 {
            let stop = pool + idx as u32 * ROUTE_STOP_SIZE;
            towns.push(*((stop + 2) as *const u8));
            let next = *(stop as *const u16);
            if next == idx || next >= pool_count || next == head {
                break;
            }
            idx = next;
        }
    }
    if towns.len() < 2 {
        notify("route time probe: the ship has no route with at least two stops");
        return;
    }

    let class35 = p3_api::class35::Class35Ptr::new();
    let mut total_time: u32 = 0;
    let mut total_distance: i64 = 0;
    let mut lines = Vec::new();
    for i in 0..towns.len() {
        let from = towns[i];
        let to = towns[(i + 1) % towns.len()];
        if from == to {
            continue;
        }
        let name = |t: u8| get_town_name(t).unwrap_or_else(|| format!("town {t}"));
        let (Some(from_id), Some(to_id)) = (GAME_WORLD_PTR.find_town_id(from), GAME_WORLD_PTR.find_town_id(to)) else {
            notify("route time probe: a town has no id");
            return;
        };
        let Some(route) = class35.calculate_town_route(from_id, to_id) else {
            notify(&format!("route time probe: no route {} -> {}", name(from), name(to)));
            return;
        };
        let distance = route.calculate_distance();
        let time = route.calculate_travel_time(ship_type, FULL_HEALTH_FACTOR, FULL_LOAD_CAPACITY_FACTOR);
        route.free();
        total_time += time;
        total_distance += distance as i64;
        lines.push(format!(
            "  {} -> {}: distance {distance}, time {time} ({:.2} days if /256)",
            name(from),
            name(to),
            time as f64 / 256.0
        ));
    }
    let stops = towns.len() as u32;
    let idle = stops * IDLE_TICKS_PER_STOP;
    info!(
        "route time for ship {selected} '{}' ({ship_type:?}, full load, full health): {} legs, total distance {total_distance}, sailing {total_time} ({:.2} days if /256), + {stops} stops x {IDLE_TICKS_PER_STOP} idle = {} ({:.2} days if /256)",
        ship.get_name(),
        lines.len(),
        total_time as f64 / 256.0,
        total_time + idle,
        (total_time + idle) as f64 / 256.0
    );
    for line in &lines {
        info!("{line}");
    }
    notify(&format!(
        "Route: sailing {:.2}d + idle {:.2}d = {:.2}d (if unit is 1/256 day) -> DebugView",
        total_time as f64 / 256.0,
        idle as f64 / 256.0,
        (total_time + idle) as f64 / 256.0
    ));
}

/// The crew-rescue false negative of 30 Aug 2026, closed: the game posted "crew number
/// too low" for a different letter than assumed. Kept for the next crew question.
#[allow(dead_code)]
unsafe fn debug_probe_ship_crew() {
    let Some(index) = selected_ship_index() else {
        notify("ship crew probe: no ship selected");
        return;
    };
    let ships = ShipsPtr::new();
    let Some(ship) = ships.get_ship(index) else {
        notify("ship crew probe: index does not resolve");
        return;
    };
    let a = ship.address;
    let type_byte = *((a + 0x0e) as *const u8);
    let grade = *((a + 0x0f) as *const u8);
    let crew = *((a + 0x40) as *const u16);
    let word_3e = *((a + 0x3e) as *const u16);
    let byte_3c = *((a + 0x3c) as *const u8);
    let byte_3d = *((a + 0x3d) as *const u8);
    let byte_3f = *((a + 0x3f) as *const u8);
    let status = *((a + 0x134) as *const u16);
    let flags_136 = *((a + 0x136) as *const u16);
    let word_138 = *((a + 0x138) as *const u16);
    let min = ship.get_min_sailors();
    let berths = ship.get_free_sailor_berths();
    let line = format!(
        "ship {index} '{}' at {a:#010x}: type {type_byte:#04x} (&3={}) +0xF={grade} | crew(+0x40)={crew} min(table)={min} berths(0x5184F0)={berths} | +0x3C={byte_3c:#04x} +0x3D={byte_3d:#04x} +0x3E={word_3e} +0x3F={byte_3f:#04x} | status(+0x134)={status:#x} flags(+0x136)={flags_136:#06x} +0x138={word_138:#06x} | town(+0x39)={:?} capacity={}",
        ship.get_name(),
        type_byte & 3,
        ship.get_last_town_index(),
        ship.get_capacity(),
    );
    info!("{line}");
    notify("ship crew probe -> DebugView");
}

#[allow(dead_code)]
unsafe fn debug_probe1() {
    let mut out: Vec<String> = Vec::new();
    let now = GAME_WORLD_PTR.get_game_time_raw();
    let rank = *(0x006DE52C as *const u8);
    let local = OPERATIONS_PTR.get_player_merchant_index();
    let player = GAME_WORLD_PTR.get_merchant(local as u16);
    out.push(format!(
        "=== task-0x2C dump | tick {now} ({:#010x}) year {} day {} | difficulty global {rank} | player money {} company value {} ===",
        now,
        GAME_WORLD_PTR.get_year(),
        GAME_WORLD_PTR.get_day_of_year(),
        player.get_money(),
        player.get_company_value(),
    ));

    let array = *(0x006DD73C as *const u32);
    let count = *(0x006DD748 as *const u16) as u32;
    if !p3_api::memory::is_readable(array, count as usize * 0x18) {
        notify("task probe: collection unreadable");
        return;
    }
    let mut found = 0;
    for i in 0..count {
        let rec = array + i * 0x18;
        let kind = *((rec + 0x6) as *const u16);
        if kind != 0x2C {
            continue;
        }
        found += 1;
        let due = *(rec as *const u32);
        let w4 = *((rec + 0x4) as *const u16);
        let d8 = *((rec + 0x8) as *const u32);
        let dc = *((rec + 0xc) as *const u32);
        let d10 = *((rec + 0x10) as *const u32);
        let d14 = *((rec + 0x14) as *const u32);
        // What +0x8 looks like: a date serial sits within two quarters below "now";
        // a wealth figure tracks the company value. Say which, per press.
        let looks = if d8 <= now && now.wrapping_sub(d8) <= 2 * 0x5D00 {
            "date-like"
        } else {
            "NOT date-like (wealth?)"
        };
        out.push(format!(
            "slot {i} at {rec:#010x}: due {due} (in {:.1} days) w+4 {w4} | +0x8 {d8} ({looks}) +0xC {dc} +0x10 {d10} +0x14 {d14}",
            (due as i64 - now as i64) as f64 / 256.0,
        ));
    }
    out.push(format!("{found} record(s) of kind 0x2C among {count} tasks"));
    for line in &out {
        info!("{line}");
    }
    append_probe_log("_probe_task2c.log", &out);
    notify(&format!("task 0x2C: {found} record(s) dumped"));
}

/// Append lines to a probe log in the game folder, never truncating.
fn append_probe_log(path: &str, lines: &[String]) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        for l in lines {
            let _ = writeln!(f, "[unix {stamp}] {l}");
        }
    }
}

/// How often F9 has been pressed since the DLL was loaded: press 0 starts a fresh
/// `_probe1.log`, later presses append their own block.
#[allow(dead_code)]
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

/// The convoy array, from `done/hotkey-supply.md`: records of [CONVOY_SIZE] bytes, and
/// `convoy+0x10` is the LEAD SHIP INDEX - the ship that carries the convoy's trade route
/// at [SHIP_ROUTE_HEAD_OFFSET]. `ship+0x8` is the ship's convoy index. This probe exists to
/// confirm those two hops before mod-auto-supply's route keys start relying on them, which
/// is needed because the ship panel reports the *clicked* member rather than the leader, so
/// pressing a route key on a convoy currently reads an empty route.
const SHIP_CONVOY_INDEX_OFFSET: u32 = 0x8;
const CONVOY_LEAD_SHIP_OFFSET: u32 = 0x10;
/// The ship's route flags; `+0x136 == 1` gates the game's own "route assigned" operation.
const SHIP_ROUTE_FLAGS_OFFSET: u32 = 0x136;

static PROBE_CONVOY_PRESSES: AtomicU32 = AtomicU32::new(0);

/// CTRL+SHIFT+F10 (THROWAWAY): who leads the selected ship's convoy, and who holds its
/// route.
///
/// Select a convoy (or any of its member ships) and press. The question it answers, in one
/// line at the end: does `convoy+0x10` name the ship that actually carries the route head,
/// so that resolving selection -> convoy -> lead is enough to make the route keys work on a
/// convoy selection?
///
/// It reaches that by three independent routes, so a disagreement is visible rather than
/// assumed:
///
/// 1. `convoy+0x10` - the candidate leader.
/// 2. A linear scan of the whole ships array for every ship whose `+0x8` names this convoy,
///    printing each one's route head. Slower than following a chain, but it cannot be
///    fooled by a link field that means something else.
/// 3. The `ship+0x6` chain from the candidate leader. **Read with suspicion**: that field
///    doubles as the ships-tick list link (`done/port-freezing.md`), so this walk is here
///    to be compared against the scan, not trusted.
unsafe fn debug_probe_convoy() {
    let ships = ShipsPtr::new();
    let ships_size = ships.get_ships_size();
    let pool_count = *ROUTE_STOP_POOL_COUNT;
    let Some(selected) = selected_ship_index() else {
        notify("probe convoy: no ship selected (a building window clears the selection)");
        return;
    };
    let Some(ship) = ships.get_ship(selected) else {
        notify(&format!("probe convoy: selected index {selected} out of range"));
        return;
    };

    let press = PROBE_CONVOY_PRESSES.fetch_add(1, Ordering::SeqCst);
    let mut out = Vec::new();
    let route_head = |index: u16| -> Option<u16> {
        let ship = ships.get_ship(index)?;
        Some(*((ship.address + SHIP_ROUTE_HEAD_OFFSET) as *const u16))
    };
    // One ship's line: name, convoy index, route head and whether that head is a real pool
    // entry - which is exactly the test mod-auto-supply's route reader applies.
    let describe = |index: u16| -> String {
        let Some(ship) = ships.get_ship(index) else {
            return format!("ship {index}: OUT OF RANGE (ships_size {ships_size})");
        };
        let head = *((ship.address + SHIP_ROUTE_HEAD_OFFSET) as *const u16);
        format!(
            "ship {index} {:?}: convoy +0x8 {} | route head +0x132 {head} ({}) | flags +0x136 {:#04x} | merchant {} | status +0x134 {:#04x}",
            ship.get_name(),
            ship.get::<u16>(SHIP_CONVOY_INDEX_OFFSET),
            if head < pool_count { "HAS ROUTE" } else { "no route" },
            ship.get::<u8>(SHIP_ROUTE_FLAGS_OFFSET),
            ship.get::<u8>(0x0),
            ship.get::<u8>(0x134),
        )
    };

    out.push(format!(
        "=== press {press} tick {} | selection reports ship {selected} | ships_size {ships_size} convoys_size {} pool_count {pool_count} ===",
        GAME_WORLD_PTR.get::<u32>(0x14),
        ships.get_convoys_size()
    ));
    out.push(format!("selected: {}", describe(selected)));

    let convoy_index = ship.get::<u16>(SHIP_CONVOY_INDEX_OFFSET);
    let Some(convoy) = ships.get_convoy(convoy_index) else {
        out.push(format!(
            "convoy index {convoy_index} is not a valid convoy (convoys_size {}) - the selected ship sails alone, so the route keys already read it correctly",
            ships.get_convoys_size()
        ));
        write_convoy_log(press, &out);
        notify(&format!("probe convoy #{press}: ship {selected} is not in a convoy -> _probe_convoy.log"));
        return;
    };

    // The candidate leader, and the whole convoy record so any other field is available
    // for a later question without a second probe.
    let lead = convoy.get::<u16>(CONVOY_LEAD_SHIP_OFFSET);
    // +0x39 is raw: a convoy at sea holds a sentinel there, not a town, so print the value
    // beside the name rather than trusting it. (Handing it to get_town_name unguarded is
    // what crashed the first version of this probe - p3-api now bounds the lookup.)
    let convoy_town = convoy.get_current_town_index();
    out.push(format!(
        "convoy {convoy_index} at {:#010x}: lead +0x10 {lead} | status +0x12 {:#06x} | town +0x39 {convoy_town:#06x} ({})",
        convoy.address,
        convoy.get_status(),
        get_town_name(convoy_town as u8).unwrap_or_else(|| "not a town".into())
    ));
    for row in 0..(CONVOY_SIZE + 15) / 16 {
        let base = row * 16;
        let bytes: Vec<String> = (0..16u32)
            .filter(|i| base + i < CONVOY_SIZE)
            .map(|i| format!("{:02x}", convoy.get::<u8>(base + i)))
            .collect();
        out.push(format!("  convoy +{base:#04x}  {}", bytes.join(" ")));
    }
    out.push(format!("lead candidate: {}", describe(lead)));

    // Route 2: the linear scan. Also the answer to "is the leader the ONLY member with a
    // route head", which decides whether the lookup can be trusted blind.
    let mut members = Vec::new();
    let mut route_holders = Vec::new();
    for index in 0..ships_size {
        let Some(member) = ships.get_ship(index) else { continue };
        if member.get::<u16>(SHIP_CONVOY_INDEX_OFFSET) != convoy_index {
            continue;
        }
        members.push(index);
        if route_head(index).is_some_and(|head| head < pool_count) {
            route_holders.push(index);
        }
    }
    out.push(format!("scan: {} member ship(s) with convoy index {convoy_index}", members.len()));
    for &index in &members {
        out.push(format!("  member {}", describe(index)));
    }

    // Route 3: the +0x6 chain from the leader, capped and compared against the scan.
    let mut chain = Vec::new();
    let mut cursor = lead;
    for _ in 0..64 {
        if cursor >= ships_size || chain.contains(&cursor) {
            break;
        }
        chain.push(cursor);
        let Some(member) = ships.get_ship(cursor) else { break };
        cursor = member.get_next_ship_in_convoy();
    }
    out.push(format!(
        "+0x6 chain from lead {lead}: [{}] (terminator {cursor}) - {}",
        chain.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(", "),
        if chain.len() == members.len() && chain.iter().all(|i| members.contains(i)) {
            "AGREES with the scan"
        } else {
            "DISAGREES with the scan - +0x6 is also the ships-tick list link, so prefer the scan"
        }
    ));

    // The verdict, which is the whole point of the press.
    let verdict = match route_holders.as_slice() {
        [] => format!("NO MEMBER HAS A ROUTE HEAD - this convoy has no applied route, so nothing to compare (give it a route first)"),
        [only] if *only == lead => format!("CONFIRMED: convoy+0x10 ({lead}) is the sole route holder - selection -> +0x8 -> +0x10 is the fix"),
        holders if holders.contains(&lead) => format!(
            "PARTIAL: convoy+0x10 ({lead}) holds a route, but so do {:?} - the lookup works, but the route is not unique to the leader",
            holders.iter().filter(|&&i| i != lead).collect::<Vec<_>>()
        ),
        holders => format!("REFUTED: convoy+0x10 says {lead}, but the route head is on {holders:?} - +0x10 is not the leader, or the leader is not the route holder"),
    };
    out.push(verdict.clone());
    out.push(format!(
        "selection was {}the leader{}",
        if selected == lead { "" } else { "NOT " },
        if selected == lead {
            " - press again with a member ship selected to see the case the route keys hit"
        } else {
            ""
        }
    ));

    write_convoy_log(press, &out);
    notify(&format!("probe convoy #{press}: {verdict} -> _probe_convoy.log"));
}

/// Both the log file and DebugView, appended across presses so two selections compare
/// directly.
unsafe fn write_convoy_log(press: u32, out: &[String]) {
    for line in out {
        debug!("probe convoy: {line}");
    }
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .append(press > 0)
        .truncate(press == 0)
        .open("_probe_convoy.log");
    if let Ok(mut file) = file {
        use std::io::Write;
        for line in out {
            let _ = writeln!(file, "{line}");
        }
    }
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
static SHIPS_TICK_HOOK_PTR: AtomicPtr<CallRel32Hook> = AtomicPtr::new(std::ptr::null_mut());
static TIMELINE_LOG: Mutex<Option<std::fs::File>> = Mutex::new(None);

/// The ships tick carries more than one sampler now - the pirate timeline and the d3d9
/// list watch - so the hook is installed once and shared, and each sampler gates itself
/// on its own flag. Two `hook_call_rel32` calls on one site would fight over it.
unsafe fn ensure_ships_tick_hook() -> Result<(), String> {
    if !SHIPS_TICK_HOOK_PTR.load(Ordering::SeqCst).is_null() {
        return Ok(());
    }
    match hook_call_rel32(SHIPS_TICK_CALL_OFFSET, ships_tick_hook as usize as u32) {
        Ok(hook) => {
            SHIPS_TICK_HOOK_PTR.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst);
            Ok(())
        }
        Err(e) => Err(format!("hooking the ships tick failed: {e:?}")),
    }
}

unsafe fn toggle_probe1_timeline() {
    let on = !TIMELINE_ON.load(Ordering::SeqCst);
    if on {
        if let Err(reason) = ensure_ships_tick_hook() {
            error!("probe1 timeline: {reason}");
            notify("probe1 timeline: hook failed");
            return;
        }
        *TIMELINE_LOG.lock().unwrap() = std::fs::File::create("_probe1_timeline.log").ok();
    } else {
        *TIMELINE_LOG.lock().unwrap() = None;
    }
    TIMELINE_ON.store(on, Ordering::SeqCst);
    notify(&format!("probe1 timeline: {}", if on { "on -> _probe1_timeline.log" } else { "off" }));
}

/// Wraps the ships tick: the original first, so every sampler sees the state the tick
/// left behind.
#[no_mangle]
unsafe extern "thiscall" fn ships_tick_hook(this: u32, tick: u32) {
    let orig: extern "thiscall" fn(u32, u32) = mem::transmute((*SHIPS_TICK_HOOK_PTR.load(Ordering::Relaxed)).old_absolute);
    orig(this, tick);
    if TIMELINE_ON.load(Ordering::Relaxed) {
        sample_probe1_timeline(tick);
    }
    if D3D9_WATCH_ON.load(Ordering::Relaxed) {
        sample_d3d9_list(tick);
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

// ---------------------------------------------------------------------------------------
// The d3d9 resource-list detector
// ---------------------------------------------------------------------------------------

/// GOG's DirectDraw -> D3D9 wrapper (`DDRAW.dll`, 1.5 MB - **not** the game's own
/// `ddraw_Dll.dll`), module-relative. It prefers `0x18000000` and has loaded there in
/// every report, but the handle is looked up anyway.
const DDRAW_SURFACE_TABLE: u32 = 0x46e350;
/// High-water mark (max used index + 1), never decremented.
const DDRAW_SURFACE_HIGH_WATER: u32 = 0x49f638;
/// The table is a fixed 50,000-slot array; the high-water mark is trusted only up to it.
const DDRAW_SURFACE_TABLE_SLOTS: u32 = 50_000;
/// The eight D3D9 objects a wrapper surface owns, released as an unrolled bank by both
/// `Release()` (`DDRAW+0x26de0`) and the `{02020202}` lost-device command
/// (`DDRAW+0x263c0`). The 22 Aug crash died releasing `+0x8b4`, the 28 Aug one `+0x8bc`.
const DDRAW_SURFACE_D3D9_SLOTS: [u32; 8] = [0x8a8, 0x8ac, 0x8b0, 0x8b4, 0x8b8, 0x8bc, 0x8c0, 0x8c4];

/// `d3d9.dll` object layout, read out of its own destructor chain rather than guessed:
///
/// - `0x10047e46`, the resource destructor, opens `mov edi,ecx / mov ecx,[edi+0x14]` - so
///   the COM object the wrapper holds is `edi` and the internal resource is at `+0x14`;
/// - `0x10062d47` (destroy resource) takes a `{ ?, resource }` descriptor, loads the
///   resource from `+0x4` and the owning device from `resource+0x44`;
/// - `0x10062de9` compares `[device+0x3c]` against the resource to decide whether it is
///   the list head, then unlinks through `resource+0x78` (`next`) and `+0x7c` (`prev`).
///   The store to `next->prev` at `0x10062df8` is the crashing instruction.
/// Kept although nothing reads it any more: it is verified RE, and the story is the
/// point. The first detector went `com+0x14 -> resource -> +0x44 -> device` because that
/// is exactly what d3d9's destructor does - and it found nothing, because the wrapper had
/// no COM resource to hand at the time of the press. The lesson was to *validate* a
/// candidate rather than assume a path to it, which is what [device_check] does now.
#[allow(dead_code)]
const D3D9_COM_RESOURCE: u32 = 0x14;
const D3D9_RES_FORMAT: u32 = 0x14;
const D3D9_RES_WIDTH: u32 = 0x1c;
const D3D9_RES_HEIGHT: u32 = 0x20;
const D3D9_RES_DEVICE: u32 = 0x44;
/// The dword the misaligned write clobbers on its way past. `0` on every healthy node
/// seen so far; when the signature hits, its top byte is the missing low byte of `next`.
const D3D9_RES_PAD: u32 = 0x74;
const D3D9_RES_NEXT: u32 = 0x78;
const D3D9_RES_PREV: u32 = 0x7c;
const D3D9_DEV_LIST_HEAD: u32 = 0x3c;

/// Stop walking here. A healthy list is orders of magnitude shorter, so reaching this
/// means the links form a cycle - itself a finding.
const LIST_WALK_CAP: u32 = 65_536;
/// Enough to characterise the damage without writing a novel into the log.
const MAX_FAULTS: usize = 16;
const D3D9_LIST_LOG: &str = "_probe_d3d9_list.log";

/// Number of regions [Guarded] remembers. `VirtualQuery` per read would make a
/// thousand-node walk far too slow to run every tick, and a scan over a 4 MB data section
/// slower still, so the outcome of each query is kept - **misses included**, which is what
/// makes a wide scan affordable: free and reserved regions are huge, so one query rules
/// out megabytes of candidates at a time.
const GUARD_CACHE: usize = 64;
/// A press must not hang the game if a scan goes wrong.
const GUARD_MAX_QUERIES: u32 = 400_000;

/// Guarded reads with a per-walk region cache. Built fresh for every check so a cached
/// region can never outlive its commit: within one walk nothing is decommitted under us,
/// across walks nothing is assumed.
struct Guarded {
    regions: [(u32, u32, bool); GUARD_CACHE],
    next: usize,
    queries: u32,
}

impl Guarded {
    fn new() -> Self {
        Self { regions: [(0, 0, false); GUARD_CACHE], next: 0, queries: 0 }
    }

    /// True when `addr .. addr+len` is committed and readable.
    unsafe fn readable_len(&mut self, addr: u32, len: u32) -> bool {
        let last = addr.wrapping_add(len - 1);
        if last < addr {
            return false;
        }
        for &(base, end, ok) in self.regions.iter() {
            if end != 0 && addr >= base && last < end {
                return ok;
            }
        }
        if self.queries >= GUARD_MAX_QUERIES {
            return false;
        }
        let mut mbi: MEMORY_BASIC_INFORMATION = core::mem::zeroed();
        self.queries += 1;
        if VirtualQuery(Some(addr as *const core::ffi::c_void), &mut mbi, core::mem::size_of::<MEMORY_BASIC_INFORMATION>()) == 0 {
            return false;
        }
        let ok = mbi.State == MEM_COMMIT && mbi.Protect.0 != 0 && mbi.Protect.0 & (PAGE_NOACCESS.0 | PAGE_GUARD.0) == 0;
        let base = mbi.BaseAddress as u32;
        let end = base.wrapping_add(mbi.RegionSize as u32);
        if end > base {
            self.regions[self.next] = (base, end, ok);
            self.next = (self.next + 1) % GUARD_CACHE;
        }
        ok && last < end
    }

    unsafe fn u32(&mut self, addr: u32) -> Option<u32> {
        if !self.readable_len(addr, 4) {
            return None;
        }
        Some(core::ptr::read_unaligned(addr as *const u32))
    }

    /// Only used by the device signature test, so a small fixed buffer is plenty.
    unsafe fn bytes(&mut self, addr: u32, out: &mut [u8]) -> bool {
        if !self.readable_len(addr, out.len() as u32) {
            return false;
        }
        core::ptr::copy_nonoverlapping(addr as *const u8, out.as_mut_ptr(), out.len());
        true
    }
}

/// The cheap arithmetic screen, and the reason the detector needs no dereference to spot
/// the fault: NT heap user blocks are 8-byte aligned and live well above the first 64 KB,
/// so both known-bad values fail here - `0x00009138` (28 Aug) on the range, `0x00008f3e`
/// (22 Aug) on the alignment too.
fn plausible_ptr(p: u32) -> bool {
    p >= 0x0001_0000 && p < 0x8000_0000 && p & 7 == 0
}

struct ListVerdict {
    head: u32,
    nodes: u32,
    faults: Vec<String>,
    truncated: bool,
    /// The first back-link fault, structurally: `(node, true_predecessor)`. This is the
    /// shape the 29 Aug live capture had - a periodic decrement of `node+0x7C` - and it is
    /// what the hardware write watch arms on: the writer demonstrably comes back.
    back_link: Option<(u32, u32)>,
}

impl ListVerdict {
    /// An empty list is **not** a pass. A device that owns no resources has had nothing
    /// checked, and saying "clean" there is how a stale device hides everything behind it -
    /// which is exactly what happened on the first live run, in the menu, right after the
    /// device had been torn down.
    fn checked_nothing(&self) -> bool {
        self.head == 0 || self.nodes == 0
    }

    fn clean(&self) -> bool {
        !self.checked_nothing() && self.faults.is_empty() && !self.truncated
    }

    /// Nothing / clean / broken, for the watch's coarse transitions - a slide into
    /// "nothing to check" is reported rather than blending into a pass.
    fn kind(&self) -> u32 {
        if self.checked_nothing() {
            0
        } else if self.clean() {
            1
        } else {
            2
        }
    }

    /// Hash of the full fault text, so the watch logs when a fault *changes*, not only
    /// when one appears. The 29 Aug capture lost the corrupted value's drift
    /// (`...07 -> ...06`) between daily heartbeats because the old key was only the
    /// fault count; the drift's timestamp was the cadence evidence, and it was gone.
    fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.truncated.hash(&mut h);
        self.checked_nothing().hash(&mut h);
        self.faults.hash(&mut h);
        h.finish()
    }
}

/// Walks `device+0x3c` and checks the doubly-linked invariant in one pass: every node's
/// `prev` must be the node the walk arrived from, every node must belong to this device,
/// and every `next` must be a plausible pointer. That is the whole of what
/// `0x10062df8` relies on.
unsafe fn walk_resource_list(g: &mut Guarded, device: u32) -> ListVerdict {
    let mut v = ListVerdict { head: 0, nodes: 0, faults: Vec::new(), truncated: false, back_link: None };
    let Some(head) = g.u32(device + D3D9_DEV_LIST_HEAD) else {
        v.faults.push(format!("device {device:#010x}: the list head at +0x3c is unreadable"));
        return v;
    };
    v.head = head;
    let mut node = head;
    let mut arrived_from = 0u32;
    while node != 0 {
        if !plausible_ptr(node) {
            v.faults.push(format!(
                "node #{}: the link from {arrived_from:#010x} points at {node:#010x}, not a plausible heap pointer",
                v.nodes
            ));
            break;
        }
        let fields = (
            g.u32(node + D3D9_RES_PREV),
            g.u32(node + D3D9_RES_NEXT),
            g.u32(node + D3D9_RES_DEVICE),
            g.u32(node + D3D9_RES_PAD),
        );
        let (prev, next, owner, pad) = match fields {
            (Some(a), Some(b), Some(c), Some(d)) => (a, b, c, d),
            _ => {
                v.faults.push(format!("node #{} at {node:#010x} is not fully readable", v.nodes));
                break;
            }
        };
        if prev != arrived_from {
            if v.back_link.is_none() {
                v.back_link = Some((node, arrived_from));
            }
            v.faults.push(format!(
                "node #{} at {node:#010x}: prev = {prev:#010x}, but the walk arrived from {arrived_from:#010x} - back link broken",
                v.nodes
            ));
        }
        if owner != device {
            v.faults.push(format!(
                "node #{} at {node:#010x}: +0x44 device = {owner:#010x}, expected {device:#010x} - foreign or recycled block",
                v.nodes
            ));
        }
        if next != 0 && !plausible_ptr(next) {
            let what = describe_resource(g, node);
            let hint = misaligned_hint(g, node, pad);
            v.faults.push(format!(
                "node #{} at {node:#010x}: next = {next:#010x} IS WILD - THIS is the node that crashes d3d9. \
                 {what}, +0x74 = {pad:#010x}{hint}",
                v.nodes
            ));
            break;
        }
        arrived_from = node;
        node = next;
        v.nodes += 1;
        if v.nodes >= LIST_WALK_CAP {
            v.truncated = true;
            break;
        }
        if v.faults.len() >= MAX_FAULTS {
            break;
        }
    }
    v
}

unsafe fn describe_resource(g: &mut Guarded, node: u32) -> String {
    match (g.u32(node + D3D9_RES_FORMAT), g.u32(node + D3D9_RES_WIDTH), g.u32(node + D3D9_RES_HEIGHT)) {
        (Some(f), Some(w), Some(h)) => format!("format {f:#x} {w}x{h}"),
        _ => "descriptor fields unreadable".to_string(),
    }
}

/// Tests the crash's own signature on the spot. If a dword was stored at `node+0x77`
/// instead of `+0x78`, the four bytes read from `+0x77` are still the pointer that was
/// meant to go in, and `+0x74`'s top byte is its low byte. Confirming or refuting that
/// per fault costs one unaligned read, so there is no reason not to.
unsafe fn misaligned_hint(g: &mut Guarded, node: u32, pad: u32) -> String {
    match g.u32(node + D3D9_RES_NEXT - 1) {
        Some(shifted) if plausible_ptr(shifted) && pad >> 24 != 0 => {
            format!(" - the dword at +0x77 reads {shifted:#010x}, a plausible pointer: SIGNATURE MATCHES, something stored it one byte low")
        }
        Some(shifted) => {
            format!(" - the dword at +0x77 reads {shifted:#010x}, not a plausible pointer: signature does NOT match, this is a different corruption")
        }
        None => String::new(),
    }
}

/// Two independent ways to recognise d3d9's internal device struct, either of which is
/// conclusive on its own - so an empty resource list does not hide the device, and a
/// device with no display name is still found by the list.
///
/// - the **round trip**: its `+0x3c` list head is a resource whose `+0x44` points back at
///   it. Nothing else in memory does that by accident;
/// - the **display name**: `device+0xc` holds the adapter's device name, `"\\.\DISPLAY1"`
///   in the 28 Aug report. Not a guess about layout so much as a fingerprint.
/// The two tests are **not** equally strong, which a live run made plain: after ESC into
/// the menu the cached device still carried its display name while its resource list had
/// been emptied, so it validated and the walk then reported "clean: 0 resources" - a check
/// of nothing, dressed up as a pass. The round trip proves the device is *live*; the
/// display name only proves the struct is *a* d3d9 device, dead or alive.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DeviceProof {
    /// `device+0x3c` is a resource whose `+0x44` points back. Conclusive, and only true
    /// of a device that currently owns resources.
    RoundTrip,
    /// `device+0xc` holds the adapter name (`"\\.\DISPLAY1"` in the 28 Aug report). Weaker:
    /// a torn-down device keeps it.
    DisplayName,
}

unsafe fn device_check(g: &mut Guarded, device: u32) -> Option<DeviceProof> {
    if !plausible_ptr(device) {
        return None;
    }
    if let Some(head) = g.u32(device + D3D9_DEV_LIST_HEAD) {
        if plausible_ptr(head) && g.u32(head + D3D9_RES_DEVICE) == Some(device) {
            return Some(DeviceProof::RoundTrip);
        }
    }
    let mut name = [0u8; 11];
    if g.bytes(device + 0xc, &mut name) && &name == b"\\\\.\\DISPLAY" {
        return Some(DeviceProof::DisplayName);
    }
    None
}

/// `strict` demands a live device. Discovery runs strict first and only falls back so that
/// a genuinely idle device still gets reported rather than looking like "not found".
unsafe fn device_accepts(g: &mut Guarded, device: u32, strict: bool) -> bool {
    match device_check(g, device) {
        Some(DeviceProof::RoundTrip) => true,
        Some(DeviceProof::DisplayName) => !strict,
        None => false,
    }
}

/// The d3d9 internal device struct, found the first time and re-validated on every use -
/// a real mode change recreates the device, so a cached pointer goes stale.
static D3D9_DEVICE: AtomicU32 = AtomicU32::new(0);

/// Bumped every time the device pointer changes, which is worth counting rather than
/// inferring: d3d9 destroys and recreates the device on a real mode change, so this is a
/// direct measure of how often the game takes the path the crash died on.
static D3D9_DEVICE_GENERATION: AtomicU32 = AtomicU32::new(0);

unsafe fn d3d9_device(g: &mut Guarded) -> Result<(u32, String), String> {
    // Only a device that still passes the round trip may be reused: a cached pointer that
    // now validates on the display name alone is exactly the stale-device trap above.
    let cached = D3D9_DEVICE.load(Ordering::Relaxed);
    if cached != 0 && device_check(g, cached) == Some(DeviceProof::RoundTrip) {
        return Ok((cached, "cached".to_string()));
    }
    let (device, how) = find_d3d9_device(g)?;
    D3D9_DEVICE.store(device, Ordering::Relaxed);
    let generation = D3D9_DEVICE_GENERATION.fetch_add(1, Ordering::Relaxed);
    let route = if cached == 0 {
        format!("{how}, first device of the session")
    } else if cached == device {
        // Revalidation failed but the same pointer came back: the device did not change,
        // its list was momentarily unreadable or empty. Worth distinguishing.
        format!("{how}, same device re-validated (generation {generation})")
    } else {
        format!("{how}, DEVICE REPLACED - was {cached:#010x} (generation {generation}, so the device has been recreated {generation} time(s) this session)")
    };
    Ok((device, route))
}

/// Wrapper globals that hold a D3D9 object, harvested from the lost-device recovery
/// routine at `DDRAW+0x200e5` - the one place that touches all of them in a row. The
/// device itself is first; the rest are whatever the wrapper keeps alive across a reset,
/// and any of them is a usable starting point because they all lead to the same device.
const DDRAW_OBJECT_GLOBALS: [(u32, &str); 5] = [
    (0x459500, "device global"),
    (0x16f6a8, "object global +0x16f6a8"),
    (0x16f6a4, "object global +0x16f6a4"),
    (0x44c834, "object global +0x44c834"),
    (0x4591d4, "object global +0x4591d4"),
];
/// The wrapper's second object table (`DDRAW+0x46120c`, count at `+0x16f660`), released
/// beside the surfaces in the same routine.
const DDRAW_OBJECT_TABLE: u32 = 0x46120c;
const DDRAW_OBJECT_TABLE_COUNT: u32 = 0x16f660;
const DDRAW_OBJECT_TABLE_CAP: u32 = 256;
/// How far into a COM object to look for the fields that lead to the device.
const COM_SCAN_BYTES: u32 = 0x100;

/// Everything a candidate COM object might be, all of it validated rather than assumed -
/// which is the lesson of the first attempt, where `com+0x14` was read correctly out of
/// d3d9's destructor but simply was not reachable from what the wrapper had to hand.
unsafe fn device_from_candidate(g: &mut Guarded, com: u32, strict: bool) -> Option<(u32, &'static str)> {
    if !plausible_ptr(com) {
        return None;
    }
    // The candidate is itself the device.
    if device_accepts(g, com, strict) {
        return Some((com, "global is the device"));
    }
    // The candidate is a d3d9 resource: +0x44 is its device.
    if let Some(device) = g.u32(com + D3D9_RES_DEVICE) {
        if device_accepts(g, device, strict) {
            return Some((device, "global is a resource"));
        }
    }
    for offset in (0..COM_SCAN_BYTES).step_by(4) {
        // A field that is the device, or that is a resource pointing at it.
        if let Some(field) = g.u32(com + offset) {
            if device_accepts(g, field, strict) {
                return Some((field, "device in a COM field"));
            }
            if plausible_ptr(field) {
                if let Some(device) = g.u32(field + D3D9_RES_DEVICE) {
                    if device_accepts(g, device, strict) {
                        return Some((device, "resource in a COM field"));
                    }
                }
            }
        }
        // The device embedded in the candidate rather than pointed at.
        if device_accepts(g, com + offset, strict) {
            return Some((com + offset, "device embedded in the COM object"));
        }
    }
    None
}

unsafe fn find_d3d9_device(g: &mut Guarded) -> Result<(u32, String), String> {
    // A live device first. Only if there is none does an idle one count, and then the
    // report says so, because "0 resources" must never read as a passed check.
    match find_d3d9_device_pass(g, true) {
        Ok(found) => Ok(found),
        Err(strict_error) => match find_d3d9_device_pass(g, false) {
            Ok((device, how)) => Ok((device, format!("{how}, IDLE DEVICE - it owns no resources"))),
            Err(_) => Err(strict_error),
        },
    }
}

unsafe fn find_d3d9_device_pass(g: &mut Guarded, strict: bool) -> Result<(u32, String), String> {
    let base = ddraw_base()?;

    for (offset, what) in DDRAW_OBJECT_GLOBALS {
        if let Some(value) = g.u32(base + offset) {
            if let Some((device, how)) = device_from_candidate(g, value, strict) {
                return Ok((device, format!("{what} -> {how}")));
            }
        }
    }

    let surfaces = g.u32(base + DDRAW_SURFACE_HIGH_WATER).unwrap_or(0).min(DDRAW_SURFACE_TABLE_SLOTS);
    for index in 0..surfaces {
        let Some(surface) = g.u32(base + DDRAW_SURFACE_TABLE + index * 4) else { continue };
        if !plausible_ptr(surface) {
            continue;
        }
        for slot in DDRAW_SURFACE_D3D9_SLOTS {
            let Some(com) = g.u32(surface + slot) else { continue };
            if let Some((device, how)) = device_from_candidate(g, com, strict) {
                return Ok((device, format!("surface table slot {index}/{slot:#x} -> {how}")));
            }
        }
    }

    let objects = g.u32(base + DDRAW_OBJECT_TABLE_COUNT).unwrap_or(0).min(DDRAW_OBJECT_TABLE_CAP);
    for index in 0..objects {
        let Some(com) = g.u32(base + DDRAW_OBJECT_TABLE + index * 4) else { continue };
        if let Some((device, how)) = device_from_candidate(g, com, strict) {
            return Ok((device, format!("object table entry {index} -> {how}")));
        }
    }

    // Last resort: the device pointer is stored *somewhere* in the wrapper's data, so
    // sweep it. Affordable only because [Guarded] caches misses - the free regions the
    // garbage points into are ruled out a megabyte at a time.
    if let Some((from, device, how)) = scan_ddraw_data(g, base, strict) {
        return Ok((device, format!("data scan at {from:#010x} -> {how}")));
    }

    Err(format!(
        "no d3d9 device found (DDRAW at {base:#010x}, {surfaces} surface slots, {objects} object slots, {} VirtualQuery)",
        g.queries
    ))
}

/// Walks DDRAW's writable sections looking for a stored pointer that validates as the
/// device. Section bounds are read from the loaded PE headers rather than hardcoded.
unsafe fn scan_ddraw_data(g: &mut Guarded, base: u32, strict: bool) -> Option<(u32, u32, &'static str)> {
    for (start, size) in writable_sections(g, base) {
        let mut addr = start;
        let end = start.wrapping_add(size);
        while addr < end {
            if let Some(value) = g.u32(addr) {
                if plausible_ptr(value) && device_accepts(g, value, strict) {
                    return Some((addr, value, "stored device pointer"));
                }
            }
            addr = addr.wrapping_add(4);
            if g.queries >= GUARD_MAX_QUERIES {
                return None;
            }
        }
    }
    None
}

/// `(virtual address, virtual size)` of every writable section of a loaded module.
unsafe fn writable_sections(g: &mut Guarded, base: u32) -> Vec<(u32, u32)> {
    const IMAGE_SCN_MEM_WRITE: u32 = 0x8000_0000;
    let mut out = Vec::new();
    let Some(pe_offset) = g.u32(base + 0x3c) else { return out };
    let pe = base.wrapping_add(pe_offset);
    if g.u32(pe) != Some(0x0000_4550) {
        return out;
    }
    // NumberOfSections is the u16 at pe+0x6, SizeOfOptionalHeader the u16 at pe+0x14 -
    // both the *low* half of the dword that contains them.
    let (Some(sections), Some(opt_size)) = (g.u32(pe + 0x6).map(|v| v & 0xFFFF), g.u32(pe + 0x14).map(|v| v & 0xFFFF)) else {
        return out;
    };
    let table = pe.wrapping_add(0x18).wrapping_add(opt_size);
    for index in 0..sections.min(32) {
        let header = table.wrapping_add(index * 40);
        let (Some(vsize), Some(rva), Some(flags)) = (g.u32(header + 0x8), g.u32(header + 0xc), g.u32(header + 0x24)) else {
            continue;
        };
        if flags & IMAGE_SCN_MEM_WRITE != 0 && vsize > 0 && vsize < 0x0400_0000 {
            out.push((base.wrapping_add(rva), vsize));
        }
    }
    out
}

unsafe fn ddraw_base() -> Result<u32, String> {
    let module = GetModuleHandleA(s!("DDRAW.dll")).map_err(|e| format!("DDRAW.dll not loaded ({e})"))?;
    let base = module.0 as u32;
    if base == 0 {
        return Err("DDRAW.dll handle is null".to_string());
    }
    Ok(base)
}

/// What the wrapper actually had to hand, dumped when discovery fails so the next attempt
/// is aimed rather than guessed.
unsafe fn d3d9_diagnostics(g: &mut Guarded) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(base) = ddraw_base() else {
        out.push("DDRAW.dll is not loaded".to_string());
        return out;
    };
    out.push(format!(
        "  gate +0x459504 = {:?}, surface high-water = {:?}, object count = {:?}",
        g.u32(base + 0x459504),
        g.u32(base + DDRAW_SURFACE_HIGH_WATER),
        g.u32(base + DDRAW_OBJECT_TABLE_COUNT),
    ));
    for (offset, what) in DDRAW_OBJECT_GLOBALS {
        let value = g.u32(base + offset);
        out.push(format!("  {what} [{:#010x}] = {value:#010x?}", base + offset));
        if let Some(value) = value {
            if plausible_ptr(value) {
                out.push(format!("    {}", dump_dwords(g, value, 16)));
            }
        }
    }
    let surfaces = g.u32(base + DDRAW_SURFACE_HIGH_WATER).unwrap_or(0).min(16);
    for index in 0..surfaces {
        let surface = g.u32(base + DDRAW_SURFACE_TABLE + index * 4);
        out.push(format!("  surface[{index}] = {surface:#010x?}"));
        if let Some(surface) = surface {
            if plausible_ptr(surface) {
                let slots: Vec<String> = DDRAW_SURFACE_D3D9_SLOTS
                    .iter()
                    .map(|slot| format!("{slot:#x}={:#010x?}", g.u32(surface + slot)))
                    .collect();
                out.push(format!("    {}", slots.join(" ")));
            }
        }
    }
    out
}

unsafe fn dump_dwords(g: &mut Guarded, addr: u32, count: u32) -> String {
    let mut parts = Vec::new();
    for index in 0..count {
        match g.u32(addr + index * 4) {
            Some(value) => parts.push(format!("{value:08x}")),
            None => parts.push("????????".to_string()),
        }
    }
    format!("{addr:#010x}: {}", parts.join(" "))
}

/// CTRL+ALT+F9: one-shot check of d3d9's resource list, for the crash in
/// `.claude/notes/todo/device-lost-crash.md`.
///
/// That crash is `d3d9+0x62df8` storing through a resource's `next` link while the GOG
/// wrapper tears a surface down, and the resource is otherwise live and coherent - one
/// field was hit by a 4-byte pointer write landing at `resource+0x77` instead of `+0x78`,
/// one byte low. Waiting for the crash is hopeless: the release path runs on every
/// display-mode change and on exit, while the corrupted resource is rare, so the crash
/// needs both to coincide. The defect is static though, and d3d9 keeps every resource of
/// a device on a doubly-linked list - so a broken link is visible **at rest**, with no
/// mode switch, no release and no crash needed. Press this to check now; SHIFT+ALT+F9
/// watches continuously and timestamps the moment a link breaks.
unsafe fn debug_probe_d3d9_list() {
    let mut g = Guarded::new();
    let mut out = Vec::new();
    // The watch only drains on game ticks; the press drains too, so captured writes can
    // be pulled while the game sits paused.
    drain_write_watch_hits(&mut out);
    let (device, route) = match d3d9_device(&mut g) {
        Ok(found) => found,
        Err(reason) => {
            let line = format!("d3d9 list: {reason}");
            out.push(line.clone());
            out.extend(d3d9_diagnostics(&mut g));
            write_d3d9_log(&out);
            notify(&line);
            return;
        }
    };
    let verdict = walk_resource_list(&mut g, device);
    out.push(format!(
        "d3d9 list: device {device:#010x} (via {route}), head {:#010x}, {} resources, {} VirtualQuery",
        verdict.head, verdict.nodes, g.queries
    ));
    if verdict.truncated {
        out.push(format!("WALK TRUNCATED at {LIST_WALK_CAP} nodes - the links form a cycle"));
    }
    for fault in &verdict.faults {
        out.push(format!("FAULT {fault}"));
    }
    let headline = if verdict.checked_nothing() {
        "d3d9 list NOT CHECKED: the device owns no resources (torn down, or between modes) - press again".to_string()
    } else if verdict.clean() {
        format!("d3d9 list clean: {} resources, links consistent", verdict.nodes)
    } else {
        format!("d3d9 list BROKEN: {} fault(s) over {} resources", verdict.faults.len(), verdict.nodes)
    };
    out.push(headline.clone());
    write_d3d9_log(&out);
    notify(&format!("{headline} -> {D3D9_LIST_LOG}"));
}

/// SHIFT+ALT+F9: watch the list continuously, off the ships tick so it samples at every
/// game speed. Quiet by design - it writes only when the fault state changes, plus a
/// heartbeat, so a clean session leaves a handful of lines and the first broken link is
/// timestamped to the tick.
static D3D9_WATCH_ON: AtomicBool = AtomicBool::new(false);
/// A day between heartbeats (a day is 256 ticks).
const D3D9_WATCH_HEARTBEAT: u32 = 256;
/// Floor on wall-clock time between walks. The tick is the right *trigger* - it fires at
/// every game speed - but it is the wrong *rate*: fast-forward advances up to a whole day
/// per frame, which would run the walk 256 times in one frame. 100 ms bounds the cost
/// whatever the speed and is still far finer than any action a player can take.
const D3D9_WATCH_MIN_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);
static D3D9_WATCH_LAST: Mutex<Option<std::time::Instant>> = Mutex::new(None);

/// True at most once per [D3D9_WATCH_MIN_INTERVAL].
fn d3d9_watch_due() -> bool {
    let now = std::time::Instant::now();
    let Ok(mut last) = D3D9_WATCH_LAST.lock() else { return false };
    if last.is_some_and(|at| now.duration_since(at) < D3D9_WATCH_MIN_INTERVAL) {
        return false;
    }
    *last = Some(now);
    true
}

unsafe fn toggle_probe_d3d9_watch() {
    let on = !D3D9_WATCH_ON.load(Ordering::SeqCst);
    if on {
        if let Err(reason) = ensure_ships_tick_hook() {
            error!("d3d9 watch: {reason}");
            notify(&format!("d3d9 watch: {reason}"));
            return;
        }
    }
    D3D9_WATCH_ON.store(on, Ordering::SeqCst);
    let mut out = vec![format!("d3d9 watch: {}", if on { "on" } else { "off" })];
    let line = out[0].clone();
    if !on {
        // The write watch hangs off the walk, so it goes down with it - leaving Dr0 armed
        // with nothing draining the hit buffer would be a watch nobody reads.
        drain_write_watch_hits(&mut out);
        disarm_write_watch("watch toggled off", &mut out);
    }
    write_d3d9_log(&out);
    notify(&format!("{line} -> {D3D9_LIST_LOG}"));
}

/// Runs on every ships tick. Allocation-free while the list is clean: the walk only
/// builds strings when it finds a fault, and the heartbeat formats once a day.
unsafe fn sample_d3d9_list(tick: u32) {
    static LAST_TICK: AtomicU32 = AtomicU32::new(0);
    static LAST_KIND: AtomicU32 = AtomicU32::new(u32::MAX);
    static LAST_FINGERPRINT: AtomicU64 = AtomicU64::new(0);
    if !d3d9_watch_due() {
        return;
    }
    // Hits the Dr0 trap deferred (the VEH must not allocate or do file I/O) are formatted
    // and written here, on the same thread, in a normal context.
    let mut out = Vec::new();
    drain_write_watch_hits(&mut out);
    let mut g = Guarded::new();
    let (device, _) = match d3d9_device(&mut g) {
        Ok(found) => found,
        Err(_) => {
            if !out.is_empty() {
                write_d3d9_log(&out);
            }
            return;
        }
    };
    let verdict = walk_resource_list(&mut g, device);
    let kind = verdict.kind();
    let fingerprint = verdict.fingerprint();
    let kind_changed = LAST_KIND.swap(kind, Ordering::Relaxed) != kind;
    let text_changed = LAST_FINGERPRINT.swap(fingerprint, Ordering::Relaxed) != fingerprint;
    let changed = kind_changed || text_changed;
    let due = tick.wrapping_sub(LAST_TICK.load(Ordering::Relaxed)) >= D3D9_WATCH_HEARTBEAT;
    if !changed && !due && out.is_empty() {
        return;
    }
    LAST_TICK.store(tick, Ordering::Relaxed);
    let prefix = format!("tick {tick} day {} time {:#04x}", tick >> 8, tick & 0xFF);
    match kind {
        0 => {
            out.push(format!("{prefix} nothing to check - the device owns no resources"));
            disarm_write_watch("device owns no resources", &mut out);
        }
        1 => {
            out.push(format!("{prefix} {}clean, {} resources", if kind_changed { "RECOVERED " } else { "" }, verdict.nodes));
            // A list healed by our own repair (or by a neighbour's release rewriting the
            // link) keeps its watch: the decrementer comes back on a day cadence and must
            // still trap. Disarm only once the watched node is no longer this device's.
            let watched_node = WRITE_WATCH_NODE.load(Ordering::SeqCst);
            if WRITE_WATCH_ADDRESS.load(Ordering::SeqCst) != 0 && g.u32(watched_node + D3D9_RES_DEVICE) != Some(device) {
                disarm_write_watch("watched node is gone", &mut out);
            }
            // A clean list also retires the no-re-arm guard: it exists only to stop an
            // instant re-arm loop right after an in-trap disarm, and heap determinism
            // makes the same address a plausible future node - which must arm again.
            WRITE_WATCH_EXHAUSTED_NODE.store(0, Ordering::SeqCst);
        }
        _ => {
            // CHANGED marks a fault whose *text* moved while broken - for the periodic
            // decrementer this line is a timestamp on each write, which is the cadence
            // evidence the 29 Aug capture lost between heartbeats.
            let tag = if kind_changed {
                "FIRST SEEN "
            } else if text_changed {
                "CHANGED "
            } else {
                ""
            };
            out.push(format!(
                "{prefix} {tag}BROKEN: {} fault(s) over {} resources, device {device:#010x}",
                verdict.faults.len(),
                verdict.nodes
            ));
            if verdict.truncated {
                out.push(format!("  cycle: walk truncated at {LIST_WALK_CAP} nodes"));
            }
            for fault in &verdict.faults {
                out.push(format!("  FAULT {fault}"));
            }
            if let Some((node, arrived_from)) = verdict.back_link {
                if changed {
                    dump_link_neighbourhood(&mut g, node, arrived_from, &mut out);
                }
                // A watch left on a node the device no longer owns (rebuild, save load)
                // must not block arming on the node that is broken now.
                let watched_node = WRITE_WATCH_NODE.load(Ordering::SeqCst);
                if WRITE_WATCH_ADDRESS.load(Ordering::SeqCst) != 0
                    && watched_node != node
                    && g.u32(watched_node + D3D9_RES_DEVICE) != Some(device)
                {
                    disarm_write_watch("watched node is gone, another node is broken", &mut out);
                }
                repair_back_link(&mut g, device, node, arrived_from, &mut out);
                let exhausted = WRITE_WATCH_EXHAUSTED_NODE.load(Ordering::SeqCst) == node;
                if WRITE_WATCH_ADDRESS.load(Ordering::SeqCst) == 0 && !exhausted {
                    arm_write_watch(node, &mut out);
                }
            }
        }
    }
    write_d3d9_log(&out);
    if changed && kind == 2 {
        notify(&format!("d3d9 list BROKE at tick {tick} - see {D3D9_LIST_LOG}"));
    }
}

/// Appended, never truncated: two presses days apart, and a watch spanning a whole
/// session, all compare in one file. Each file line carries a `[unix N]` stamp in
/// `_window_lifecycle.log`'s format, so the two logs cross-reference - the 29 Aug capture
/// could not be placed against the window log for want of exactly this.
fn write_d3d9_log(lines: &[String]) {
    for line in lines {
        debug!("{line}");
    }
    let stamp = unix_now();
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(D3D9_LIST_LOG) {
        use std::io::Write;
        for line in lines {
            let _ = writeln!(file, "[unix {stamp}] {line}");
        }
        let _ = file.flush();
    }
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Option 2c: the hardware write watch (device-lost-crash.md).
//
// The 29 Aug live capture showed a corrupted `prev` link being *decremented* on a
// game-day cadence - a writer that comes back. A polling walk can prove the field
// changed but never who changed it; a Dr0 data breakpoint on the field names the
// writer's EIP on its next visit. The watch arms itself when the list walk first sees
// a broken back link, and every write to that dword after that - the culprit, d3d9's
// own legitimate unlink when a neighbour is released, the heap's free-list bookkeeping
// when the block dies - is captured with registers, code bytes and stack, each one
// labelled by the module its EIP falls in.
//
// Everything here runs on the game's main thread: the walk that arms (ships tick), the
// writers being hunted (game logic and d3d9 releases), and therefore the trap. Debug
// registers are per-thread, so arming the current thread is exactly right - and
// SetThreadContext with only CONTEXT_DEBUG_REGISTERS on the current thread is the
// standard self-debugging technique.
//
// The VEH is the delicate part: the trap can fire inside the heap's own bookkeeping
// (the freed block's list links get written), so the handler must not allocate or take
// the heap lock. It copies everything into a fixed buffer behind a try_lock and the
// next walk formats and writes it from a normal context.
// ---------------------------------------------------------------------------

/// winnt.h: `CONTEXT_i386 (0x00010000) | CONTEXT_DEBUG_REGISTERS (0x00000010)` - the
/// windows 0.48 crate does not export the x86 composite.
const CONTEXT_DEBUG_REGISTERS_I386: u32 = 0x0001_0010;
/// Dr7 slot 0: L0+G0 enable bits, plus RW0 and LEN0.
const DR7_SLOT0_MASK: u32 = 0b11 | (0b1111 << 16);
/// L0 set, RW0 = 01 (break on data writes), LEN0 = 11 (4-byte range). The watched
/// address must be 4-aligned; `node+0x7C` is (heap blocks are 8-aligned).
const DR7_SLOT0_WRITE_DWORD: u32 = 0b1 | (0b01 << 16) | (0b11 << 18);
/// Dr6 B0: slot 0 fired.
const DR6_HIT0: u32 = 1;
const EXCEPTION_SINGLE_STEP_CODE: u32 = 0x8000_0004;
const EXCEPTION_CONTINUE_EXECUTION: i32 = -1;
const EXCEPTION_CONTINUE_SEARCH: i32 = 0;
/// Distinct write sites (EIPs) the buffer holds per drain cycle. The 30 Aug capture
/// burned a raw 8-hit budget in one second on a single instruction - ntdll's memset
/// zeroing the freed node block eight times during a device rebuild - so hits now dedup
/// by EIP: a repeated site costs nothing, only its count grows.
const WRITE_WATCH_SLOTS: usize = 8;
/// The runaway valve: raw traps (dedup'd or not) before the watch disarms itself. The
/// real bound is the sampler disarming when the watched node dies; this only stops a
/// pathological hot-reuse case from trapping forever while the game is not ticking.
const WRITE_WATCH_MAX_RAW: u32 = 512;
/// Code bytes captured before EIP. A data breakpoint traps *after* the write, so EIP is
/// the **next** instruction and the writer is somewhere in these preceding bytes.
const WATCH_CODE_BACK: u32 = 16;
const WATCH_CODE_AHEAD: usize = 8;

/// The watched dword's address; 0 = disarmed. Written by the armer (walk) and by the
/// VEH when it disarms in-context after the last budgeted hit.
static WRITE_WATCH_ADDRESS: AtomicU32 = AtomicU32::new(0);
/// The node the watched dword belongs to, for the log and the re-arm guard.
static WRITE_WATCH_NODE: AtomicU32 = AtomicU32::new(0);
/// A node whose watch spent its whole hit budget: do not re-arm on it, or a hot writer
/// would re-trap on every subsequent walk of the still-broken list. A *different* node
/// faulting arms fresh.
static WRITE_WATCH_EXHAUSTED_NODE: AtomicU32 = AtomicU32::new(0);
static WRITE_WATCH_HITS: AtomicU32 = AtomicU32::new(0);
/// Hits that could not be buffered (buffer full, or the try_lock lost). Counted rather
/// than lost silently.
static WRITE_WATCH_DROPPED: AtomicU32 = AtomicU32::new(0);
static WRITE_WATCH_VEH_ON: AtomicBool = AtomicBool::new(false);

/// Everything the trap can copy without allocating, formatted later by the drain.
#[derive(Clone, Copy)]
struct WatchHit {
    unix: u64,
    hit_no: u32,
    eip: u32,
    eax: u32,
    ebx: u32,
    ecx: u32,
    edx: u32,
    esi: u32,
    edi: u32,
    ebp: u32,
    esp: u32,
    /// The watched dword's address and its value right after the write.
    watched: u32,
    value: u32,
    /// `eip-16 .. eip+8`; the write instruction ends at offset 16.
    code: [u8; (WATCH_CODE_BACK as usize) + WATCH_CODE_AHEAD],
    code_ok: bool,
    stack: [u32; 8],
    stack_n: u8,
    /// Further traps at this same EIP in the same batch - regs/stack kept from the first.
    repeats: u32,
    /// This hit spent the raw budget and the VEH disarmed Dr0 in-context.
    disarmed: bool,
}

impl WatchHit {
    const EMPTY: WatchHit = WatchHit {
        unix: 0,
        hit_no: 0,
        eip: 0,
        eax: 0,
        ebx: 0,
        ecx: 0,
        edx: 0,
        esi: 0,
        edi: 0,
        ebp: 0,
        esp: 0,
        watched: 0,
        value: 0,
        code: [0; (WATCH_CODE_BACK as usize) + WATCH_CODE_AHEAD],
        code_ok: false,
        stack: [0; 8],
        stack_n: 0,
        repeats: 0,
        disarmed: false,
    };
}

/// Fixed-size so the VEH never allocates pushing into it. Same capacity as the hit
/// budget, so nothing is droppable by size alone.
struct HitBuffer {
    hits: [WatchHit; WRITE_WATCH_SLOTS],
    n: usize,
}

static WATCH_HITS_PENDING: Mutex<HitBuffer> = Mutex::new(HitBuffer { hits: [WatchHit::EMPTY; WRITE_WATCH_SLOTS], n: 0 });

/// Programs Dr0 on the current thread; `address` 0 clears the slot. Only ever called
/// from the main thread (the ships tick), which is also the only thread that can trap.
unsafe fn program_dr0(address: u32) -> Result<(), &'static str> {
    let thread = GetCurrentThread();
    let mut ctx: CONTEXT = mem::zeroed();
    ctx.ContextFlags = CONTEXT_DEBUG_REGISTERS_I386;
    if !GetThreadContext(thread, &mut ctx).as_bool() {
        return Err("GetThreadContext failed");
    }
    ctx.Dr0 = address & !3;
    ctx.Dr6 = 0;
    ctx.Dr7 = (ctx.Dr7 & !DR7_SLOT0_MASK) | if address != 0 { DR7_SLOT0_WRITE_DWORD } else { 0 };
    if !SetThreadContext(thread, &ctx).as_bool() {
        return Err("SetThreadContext failed");
    }
    Ok(())
}

/// Arms Dr0 on the faulting node's `prev` dword. Registered VEH first (position 1, so it
/// runs before the crash reporter's own handler - which ignores single-step anyway, its
/// severity filter only passes fatal codes).
unsafe fn arm_write_watch(node: u32, out: &mut Vec<String>) {
    let address = node + D3D9_RES_PREV;
    if !WRITE_WATCH_VEH_ON.swap(true, Ordering::SeqCst) {
        AddVectoredExceptionHandler(1, Some(write_watch_veh));
    }
    WRITE_WATCH_HITS.store(0, Ordering::SeqCst);
    WRITE_WATCH_NODE.store(node, Ordering::SeqCst);
    match program_dr0(address) {
        Ok(()) => {
            WRITE_WATCH_ADDRESS.store(address, Ordering::SeqCst);
            out.push(format!(
                "  WRITE WATCH ARMED: Dr0 on the dword at {address:#010x} (node {node:#010x} +0x7c, the broken prev link) - writes are caught with EIP, dedup'd per site"
            ));
        }
        Err(reason) => out.push(format!("  write watch NOT armed: {reason}")),
    }
}

unsafe fn disarm_write_watch(reason: &str, out: &mut Vec<String>) {
    if WRITE_WATCH_ADDRESS.swap(0, Ordering::SeqCst) == 0 {
        return;
    }
    let hits = WRITE_WATCH_HITS.load(Ordering::SeqCst);
    match program_dr0(0) {
        Ok(()) => out.push(format!("  write watch disarmed ({reason}), {hits} hit(s) captured")),
        Err(e) => out.push(format!("  write watch disarm FAILED ({reason}): {e}")),
    }
}

/// The trap. Runs with the game stopped mid-instruction-stream, possibly inside the
/// heap's own code, so: no allocation, no file I/O, no logging - copy, stash, continue.
/// `VirtualQuery` (inside [Guarded]) is a plain syscall and safe here.
unsafe extern "system" fn write_watch_veh(info: *mut EXCEPTION_POINTERS) -> i32 {
    let Some(info) = info.as_mut() else {
        return EXCEPTION_CONTINUE_SEARCH;
    };
    let Some(record) = info.ExceptionRecord.as_ref() else {
        return EXCEPTION_CONTINUE_SEARCH;
    };
    if record.ExceptionCode.0 as u32 != EXCEPTION_SINGLE_STEP_CODE {
        return EXCEPTION_CONTINUE_SEARCH;
    }
    let Some(ctx) = info.ContextRecord.as_mut() else {
        return EXCEPTION_CONTINUE_SEARCH;
    };
    if ctx.Dr6 & DR6_HIT0 == 0 {
        // A single-step that is not our slot (a debugger's, or TF) - not ours to eat.
        return EXCEPTION_CONTINUE_SEARCH;
    }
    ctx.Dr6 = 0;
    let watched = WRITE_WATCH_ADDRESS.load(Ordering::SeqCst);
    let hits = WRITE_WATCH_HITS.fetch_add(1, Ordering::SeqCst) + 1;
    let disarm = watched == 0 || hits >= WRITE_WATCH_MAX_RAW;

    let mut hit = WatchHit {
        unix: unix_now(),
        hit_no: hits,
        eip: ctx.Eip,
        eax: ctx.Eax,
        ebx: ctx.Ebx,
        ecx: ctx.Ecx,
        edx: ctx.Edx,
        esi: ctx.Esi,
        edi: ctx.Edi,
        ebp: ctx.Ebp,
        esp: ctx.Esp,
        watched,
        value: 0,
        disarmed: disarm,
        ..WatchHit::EMPTY
    };
    let mut g = Guarded::new();
    if let Some(value) = g.u32(watched) {
        hit.value = value;
    }
    if ctx.Eip >= WATCH_CODE_BACK {
        let mut code = hit.code;
        hit.code_ok = g.bytes(ctx.Eip - WATCH_CODE_BACK, &mut code);
        hit.code = code;
    }
    for i in 0..hit.stack.len() {
        match g.u32(ctx.Esp.wrapping_add(4 * i as u32)) {
            Some(v) => {
                hit.stack[i] = v;
                hit.stack_n = (i + 1) as u8;
            }
            None => break,
        }
    }

    match WATCH_HITS_PENDING.try_lock() {
        Ok(mut buffer) => {
            // Dedup by write site: a burst from one instruction (a memset zeroing the
            // freed block, a hot reuse) costs one slot however long it runs.
            let n = buffer.n;
            if let Some(slot) = buffer.hits[..n].iter_mut().find(|h| h.eip == hit.eip) {
                slot.repeats += 1;
                slot.disarmed |= hit.disarmed;
            } else if n < buffer.hits.len() {
                buffer.hits[n] = hit;
                buffer.n = n + 1;
            } else {
                WRITE_WATCH_DROPPED.fetch_add(1, Ordering::SeqCst);
            }
        }
        _ => {
            WRITE_WATCH_DROPPED.fetch_add(1, Ordering::SeqCst);
        }
    }

    if disarm {
        // In-context: the kernel restores this CONTEXT on continue, clearing the slot.
        ctx.Dr7 &= !DR7_SLOT0_MASK;
        ctx.Dr0 = 0;
        WRITE_WATCH_ADDRESS.store(0, Ordering::SeqCst);
        WRITE_WATCH_EXHAUSTED_NODE.store(WRITE_WATCH_NODE.load(Ordering::SeqCst), Ordering::SeqCst);
    }
    EXCEPTION_CONTINUE_EXECUTION
}

/// Formats and hands over whatever the trap stashed - called from the walk, in a normal
/// context where allocation and file I/O are fine.
unsafe fn drain_write_watch_hits(out: &mut Vec<String>) {
    let mut drained = [WatchHit::EMPTY; WRITE_WATCH_SLOTS];
    let mut n = 0;
    if let Ok(mut buffer) = WATCH_HITS_PENDING.try_lock() {
        n = buffer.n;
        drained[..n].copy_from_slice(&buffer.hits[..n]);
        buffer.n = 0;
    }
    for hit in &drained[..n] {
        let module = crate::ffi::module_of(hit.eip)
            .map(|(name, base)| format!(" ({name}+{:#x})", hit.eip - base))
            .unwrap_or_else(|| " (no module)".to_string());
        out.push(format!(
            "[hit at unix {}] WRITE WATCH HIT #{}: eip {:#010x}{module} - the write is the instruction ENDING at eip; dword at {:#010x} now {:#010x}",
            hit.unix, hit.hit_no, hit.eip, hit.watched, hit.value
        ));
        out.push(format!(
            "  regs eax={:#010x} ebx={:#010x} ecx={:#010x} edx={:#010x} esi={:#010x} edi={:#010x} ebp={:#010x} esp={:#010x}",
            hit.eax, hit.ebx, hit.ecx, hit.edx, hit.esi, hit.edi, hit.ebp, hit.esp
        ));
        if hit.code_ok {
            let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ");
            out.push(format!(
                "  code eip-{WATCH_CODE_BACK}..eip: {} | eip..+{WATCH_CODE_AHEAD}: {}",
                hex(&hit.code[..WATCH_CODE_BACK as usize]),
                hex(&hit.code[WATCH_CODE_BACK as usize..])
            ));
        }
        if hit.stack_n > 0 {
            let dwords: Vec<String> = hit.stack[..hit.stack_n as usize].iter().map(|d| format!("{d:#010x}")).collect();
            out.push(format!("  stack at esp: {}", dwords.join(" ")));
        }
        if hit.repeats > 0 {
            out.push(format!("  (+{} more trap(s) at this same eip in this batch - regs/stack above are the first)", hit.repeats));
        }
        if hit.disarmed {
            out.push(format!("  write watch disarmed in the trap - raw budget ({WRITE_WATCH_MAX_RAW}) spent"));
        }
    }
    let dropped = WRITE_WATCH_DROPPED.swap(0, Ordering::SeqCst);
    if dropped > 0 {
        out.push(format!("  write watch: {dropped} hit(s) DROPPED (buffer full or lock contended)"));
    }
    if n > 0 {
        notify(&format!("d3d9 write watch: {n} hit(s) captured -> {D3D9_LIST_LOG}"));
    }
}

/// Rewrites a poisoned `prev` with the walk's `arrived_from` - the invariant d3d9 itself
/// maintains (the walk reached this node through the predecessor's `next`, so that is
/// what `prev` must be).
///
/// This removes the crash's precondition rather than treating a symptom: the fourth
/// occurrence (29 Aug 2026, 23:09) proved the fatal "pointer stored one byte low" is
/// **d3d9's own unlink writing through a decremented `prev`** - `prev->next = next`
/// lands at `(true_prev - 1) + 0x78 = true_prev + 0x77`, wrecking the predecessor's
/// `next`, which the next teardown then dereferences wild. With `prev` repaired, every
/// release writes where it should and the whole cascade never starts.
///
/// The hunt is unaffected: the decrementer writes through its own stale pointer
/// regardless of the field's current value, so its next visit still fires the Dr0 trap.
/// Our own repair write must not - Dr0 is lifted around it when it is armed on this
/// very dword.
unsafe fn repair_back_link(g: &mut Guarded, device: u32, node: u32, arrived_from: u32, out: &mut Vec<String>) {
    if g.u32(node + D3D9_RES_DEVICE) != Some(device) {
        out.push(format!("  NOT repaired: node {node:#010x} no longer belongs to the device"));
        return;
    }
    let Some(old) = g.u32(node + D3D9_RES_PREV) else {
        out.push(format!("  NOT repaired: node {node:#010x} +0x7c unreadable"));
        return;
    };
    let armed_here = WRITE_WATCH_ADDRESS.load(Ordering::SeqCst) == node + D3D9_RES_PREV;
    if armed_here {
        let _ = program_dr0(0);
    }
    core::ptr::write_volatile((node + D3D9_RES_PREV) as *mut u32, arrived_from);
    if armed_here {
        let _ = program_dr0(node + D3D9_RES_PREV);
    }
    out.push(format!(
        "  REPAIRED: node {node:#010x} prev {old:#010x} -> {arrived_from:#010x} (the true predecessor) - the teardown crash is defused while the watch waits"
    ));
}

/// The 16 bytes at `+0x70..+0x7F` of the faulting node and both neighbours - the dump the
/// 28 Aug crash analysis assembled by hand, automated. `+0x74` (pad), `+0x78` (next),
/// `+0x7C` (prev) all sit in this window, so the one-byte-low store and the decrement
/// shapes are both readable straight off the line.
unsafe fn dump_link_neighbourhood(g: &mut Guarded, node: u32, arrived_from: u32, out: &mut Vec<String>) {
    let next = g.u32(node + D3D9_RES_NEXT).unwrap_or(0);
    for (label, base) in [("predecessor ", arrived_from), ("faulty node ", node), ("its next    ", next)] {
        if base == 0 {
            continue;
        }
        let mut bytes = [0u8; 16];
        if g.bytes(base + 0x70, &mut bytes) {
            let groups: Vec<String> =
                bytes.chunks(4).map(|c| c.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")).collect();
            out.push(format!("  {label}{base:#010x} +0x70: {}", groups.join(" | ")));
        } else {
            out.push(format!("  {label}{base:#010x} +0x70: unreadable"));
        }
    }
}
