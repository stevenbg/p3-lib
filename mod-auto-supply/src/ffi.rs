use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicPtr, AtomicU32, Ordering};
use std::sync::Mutex;
use std::{mem, panic, ptr};

use hooklet::windows::x86::{hook_call_rel32, hook_function_pointer, CallRel32Hook, FunctionPointerHook};
use log::{debug, error, info};
use num_traits::FromPrimitive;
use p3_api::{
    auto_trader::skill_caps,
    data::{enums::WareId, office::OFFICE_SIZE, p3_ptr::P3Pointer},
    game_world::{GAME_WORLD_PTR, TICKS_PER_YEAR},
    operation::Operation,
    operations::{execute_operation, OPERATIONS_PTR},
    scheduled_tasks::{
        scheduled_task::{SCHEDULED_TASK_OPCODE_TEN_DAY_UPDATE, SCHEDULED_TASK_SIZE},
        SCHEDULED_TASKS_PTR,
    },
    ship::SHIP_SIZE,
    ships::ShipsPtr,
    town::{get_town_name, TOWN_SIZE},
    ui::{ui_ship_panel::UIShipPanelPtr, ui_trading_office_window::UITradingOfficeWindowPtr},
};
use p3_rou::{builder, TradeRouteStop};
use windows::Win32::{
    Foundation::{LPARAM, LRESULT, WPARAM},
    System::Memory::{VirtualQuery, MEMORY_BASIC_INFORMATION, MEM_COMMIT},
    System::Threading::GetCurrentThreadId,
    UI::{
        Input::KeyboardAndMouse::{GetKeyState, VIRTUAL_KEY, VK_CONTROL, VK_DELETE, VK_F1, VK_F10, VK_F11, VK_F3, VK_F4, VK_F9, VK_MENU, VK_SHIFT},
        WindowsAndMessaging::{CallNextHookEx, SetWindowsHookExW, HHOOK, WH_KEYBOARD},
    },
};

use crate::prices::{buy_price, sell_price, PriceLevel};

/// F1: set every ware without an order to BUY (produced by the town) or SELL (rest),
/// at the Center price levels (buy t1, sell t0). Ctrl+F1: provision and lock the
/// celebration goods; Alt+F1: provision and lock the building materials.
const SETUP_KEY: usize = VK_F1.0 as usize;
/// The wares Ctrl+F1 provisions and locks in the office: what a celebration needs in
/// stock.
const LOCKED_STAPLES: [WareId; 6] = [WareId::Beer, WareId::Wine, WareId::Fish, WareId::Meat, WareId::Grain, WareId::Honey];
/// What Alt+F1 provisions and locks: building materials (in-game units) covering any
/// building or ship.
const LOCKED_BUILDING_MATERIALS: [(WareId, i32); 6] = [
    (WareId::Cloth, 10),
    (WareId::Hemp, 8),
    (WareId::Pitch, 50),
    (WareId::Bricks, 80),
    (WareId::Timber, 50),
    (WareId::IronGoods, 50),
];

/// The six price levels, on Q W E R T Y in ASCENDING price order: ctrl sets buy prices,
/// alt sets sell prices. Pressed without a modifier (or with both) they do nothing.
/// Buying gets more aggressive towards Y (drains the town deeper), selling gets more
/// restrained (only sells into scarcity); R is the natural pair, buy par / sell supply.
const LEVEL_KEYS: [(usize, PriceLevel); 6] = [
    (0x51, PriceLevel::Upper),    // Q: buy t2          / sell t1
    (0x57, PriceLevel::UpperMid), // W: buy mid t1..t2  / sell mid t0..t1
    (0x45, PriceLevel::Upper30),  // E: buy 30% t1->t2  / sell 30% t0->t1
    (0x52, PriceLevel::Center),   // R: buy t1 (par)    / sell t0 (the supply price)
    (0x54, PriceLevel::Lower70),  // T: buy 70% t0->t1  / sell 70% 0->t0
    (0x59, PriceLevel::LowerMid), // Y: buy mid t0..t1  / sell mid 0..t0
];
/// F11: dump the current town's thresholds, base prices and price levels to the log and
/// to a CSV.
const TOWN_DUMP_KEY: usize = VK_F11.0 as usize;
/// F10: dump every ship's applied route chain from the route stop pool.
const ROUTE_DUMP_KEY: usize = VK_F10.0 as usize;
/// F9: throwaway debug logic for the current RE task (see [debug_probe1]). The frame
/// profiler that used to hang off Alt+F9 now lives, unused, in
/// mod-fix-texture-cache-thrash's `profiler` module.
const DEBUG_PROBE1_KEY: usize = VK_F9.0 as usize;
/// F4 appends a trade stop for the current town to the selected ship's route; Ctrl+F4
/// does the same but also buys the [NO_BUY_WARES].
const ADD_STOP_KEY: usize = VK_F4.0 as usize;
/// F3 sets a whole route template: plain 5stop, alt 6stop, ctrl suck.
const ROUTE_KEY: usize = VK_F3.0 as usize;
/// DEL clears the selected ship's route (guarded on the goods dialog being closed).
const CLEAR_ROUTE_KEY: usize = VK_DELETE.0 as usize;
/// The stop buys what the town produces at this level (the Ctrl+Y price).
const STOP_BUY_LEVEL: PriceLevel = PriceLevel::LowerMid;
/// The stop sells everything else at this level (the Alt+Y price).
const STOP_SELL_LEVEL: PriceLevel = PriceLevel::LowerMid;
/// Wares plain F4 never buys, even where the town produces them: their margin does not
/// justify the cargo space early on (grain, hemp and timber are bulky loads goods), so
/// they stay in the town. Ctrl+F4 buys them too.
const NO_BUY_WARES: [WareId; 6] = [WareId::Pitch, WareId::Timber, WareId::Salt, WareId::Bricks, WareId::Grain, WareId::Hemp];
/// Wares the F3 routes never supply, even where the town consumes them: low-value
/// industry inputs that are not worth the hold space. Whatever accumulates in the
/// target office is still hauled home. (Timber is supplied - as the worst of the loads
/// goods it is ordered last, see p3-rou's cargo_order.)
const NO_SUPPLY_WARES: [WareId; 4] = [WareId::Bricks, WareId::PigIron, WareId::Pitch, WareId::Hemp];

/// The game's route file loader: thiscall(this, base_name) -> decompressed buffer. It
/// forms the path "save\AutoRoute\<name>.rou" itself, so we write the file there and
/// pass the base name.
const ROUTE_LOADER_ADDRESS: u32 = 0x004d5ee0;
const ROUTE_LOADER_THIS: u32 = 0x006dd728;
/// operations struct (0x6df2f0): +0x930 = route buffer pointer, +0x934 = target ship.
const OPERATIONS_ROUTE_BUFFER: *mut u32 = 0x006dfc20 as _;
const OPERATIONS_ROUTE_SHIP: *mut u32 = 0x006dfc24 as _;
const ROUTE_BASE_NAME: &str = "_autosupply";
const ROUTE_FILE_PATH: &str = "save/AutoRoute/_autosupply.rou";
/// The ship's route as it was before a key changed it, so it can be loaded back through
/// the route window's File button.
const ROUTE_BACKUP_PATH: &str = "save/AutoRoute/_backup.rou";
const FIRST_STOP_MARKER: u8 = 0x04;

/// The loader takes a pointer to an MFC-style string object: a single pointer to char
/// data, with refcount/alloc/length in the 0xc bytes before it. [0x6c7cd0] holds the
/// shared empty-string header, which is what a default-constructed object points past.
const STRING_EMPTY_HEADER_PTR: *const u32 = 0x006c7cd0 as _;
/// thiscall(this, *const u8) -> this: construct/assign from a C string.
const STRING_CTOR_FROM_CSTR: u32 = 0x0064f390;
/// thiscall(this): release the string data.
const STRING_DTOR: u32 = 0x0064f253;
/// Amount set for buy orders whose current amount is 0, in in-game units.
const BUY_AMOUNT: i32 = 9999;
/// The administrator view of the trading office window ("Trading Office" side button, pages 0-6).
const ADMINISTRATOR_PAGE: i32 = 4;
/// The trade wares (weapons are not administrator-tradeable).
const TRADE_WARES: std::ops::Range<u16> = 0..20;
/// The town-scene object pointer; its +0xc324 field is the current town index.
const TOWN_SCENE_PTR: *const u32 = 0x006e51ac as _;
const TOWN_SCENE_CURRENT_TOWN_OFFSET: u32 = 0xc324;
/// The window classes share their vtable layout: +0x118 is close, +0x120 is open.
const OFFICE_WINDOW_OPEN_POINTER_OFFSET: u32 = UITradingOfficeWindowPtr::VTABLE_OFFSET + 0x120;
const OFFICE_WINDOW_CLOSE_POINTER_OFFSET: u32 = UITradingOfficeWindowPtr::VTABLE_OFFSET + 0x118;

/// Raw HHOOK of the keyboard hook (installed once at load, always active).
static KEYBOARD_HOOK: AtomicIsize = AtomicIsize::new(0);
/// True while a trading office window is open; gates F1 and the price level keys.
static OFFICE_WINDOW_OPEN: AtomicBool = AtomicBool::new(false);
static OPEN_HOOK_PTR: AtomicPtr<FunctionPointerHook> = AtomicPtr::new(std::ptr::null_mut());
static CLOSE_HOOK_PTR: AtomicPtr<FunctionPointerHook> = AtomicPtr::new(std::ptr::null_mut());

/// Post an in-game popup on the event ticker (the top-left "Game speed:" boxes),
/// mirrored to the debug log.
unsafe fn notify(text: &str) {
    info!("{text}");
    let latin1: Vec<u8> = text.chars().map(|c| if (c as u32) <= 0xff { c as u32 as u8 } else { b'?' }).collect();
    p3_api::ui::ui_notifications::UINotificationsPtr::new().post_event(&latin1);
}

#[no_mangle]
pub unsafe extern "C" fn start() -> u32 {
    let _ = log::set_logger(&win_dbg_logger::DEBUGGER_LOGGER);
    log::set_max_level(log::LevelFilter::Trace);

    panic::set_hook(Box::new(|p| {
        error!("{p}");
    }));

    // start() runs on the game's main thread (the modloader calls it from its WinMain
    // hook), which then pumps the message loop - so a thread-scoped keyboard hook
    // installed here fires on key events at any game speed, for the process lifetime.
    match SetWindowsHookExW(WH_KEYBOARD, Some(keyboard_hook), None, GetCurrentThreadId()) {
        Ok(hook) => KEYBOARD_HOOK.store(hook.0, Ordering::SeqCst),
        Err(_) => {
            error!("installing the keyboard hook failed");
            return 1;
        }
    }

    // Track whether a trading office window is open, to gate the office-only keys. The
    // OS keyboard hook is all-or-nothing per thread, so the scoping is done in code.
    match hook_function_pointer(OFFICE_WINDOW_OPEN_POINTER_OFFSET, office_window_open_hook as usize as u32) {
        Ok(hook) => OPEN_HOOK_PTR.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
        Err(_) => {
            error!("failed to hook office window open");
            return 2;
        }
    }
    match hook_function_pointer(OFFICE_WINDOW_CLOSE_POINTER_OFFSET, office_window_close_hook as usize as u32) {
        Ok(hook) => CLOSE_HOOK_PTR.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
        Err(_) => {
            error!("failed to hook office window close");
            return 3;
        }
    }

    info!("loaded: office F1 setup, ctrl/alt+QWERTY prices, F11 debug; global F4 add stop, F9 selected ship, F10 routes");
    0
}

#[no_mangle]
unsafe extern "thiscall" fn office_window_open_hook(window_address: u32) {
    let orig: extern "thiscall" fn(u32) = mem::transmute((*OPEN_HOOK_PTR.load(Ordering::SeqCst)).old_absolute);
    orig(window_address);
    OFFICE_WINDOW_OPEN.store(true, Ordering::SeqCst);
}

#[no_mangle]
unsafe extern "thiscall" fn office_window_close_hook(window_address: u32) {
    let orig: extern "thiscall" fn(u32) = mem::transmute((*CLOSE_HOOK_PTR.load(Ordering::SeqCst)).old_absolute);
    orig(window_address);
    OFFICE_WINDOW_OPEN.store(false, Ordering::SeqCst);
}

fn key_down(key: VIRTUAL_KEY) -> bool {
    (unsafe { GetKeyState(key.0 as i32) } as u16) & 0x8000 != 0
}

unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let flags = lparam.0 as u32;
        // Bit 31: transition state (0 = key pressed); bit 30: previous state (0 = was up).
        // Together: fire once on the initial key-down, not on autorepeat or release.
        if flags & 0xC000_0000 == 0 {
            let ctrl = key_down(VK_CONTROL);
            let alt = key_down(VK_MENU);
            let shift = key_down(VK_SHIFT);
            // The office keys act only while a trading office window is open; the town
            // and route probes are global.
            let office_open = OFFICE_WINDOW_OPEN.load(Ordering::SeqCst);
            match wparam.0 {
                w if w == DEBUG_PROBE1_KEY && ctrl => toggle_probe1_timeline(),
                w if w == DEBUG_PROBE1_KEY && shift => debug_probe1_ship(),
                w if w == DEBUG_PROBE1_KEY && alt => debug_probe_administrators(),
                w if w == DEBUG_PROBE1_KEY => debug_probe1(),
                w if w == ROUTE_DUMP_KEY && ctrl => debug_probe_dialog_modes(),
                w if w == ROUTE_DUMP_KEY => dump_ship_routes(),
                w if w == TOWN_DUMP_KEY => on_town_dump_hotkey(),
                // Plain F4 skips the NO_BUY_WARES, ctrl+F4 buys everything produced.
                // Alt+F4 is left to Windows.
                w if w == ADD_STOP_KEY && !alt => on_add_stop_hotkey(!ctrl),
                // Shift appends the generated route to the existing one instead of
                // replacing it.
                w if w == ROUTE_KEY => on_route_hotkey(
                    if ctrl {
                        RouteKind::Suck
                    } else if alt {
                        RouteKind::SixStop
                    } else {
                        RouteKind::FiveStop
                    },
                    shift,
                ),
                // Exactly one of ctrl (buy) / alt (sell) picks the direction. The keys
                // target the goods dialog's stop when it is open, otherwise the office
                // administrator view.
                w if ctrl != alt && LEVEL_KEYS.iter().any(|&(key, _)| key == w) => {
                    let level = LEVEL_KEYS.iter().find(|&&(key, _)| key == w).unwrap().1;
                    let (sell, buy) = if ctrl { (None, Some(level)) } else { (Some(level), None) };
                    if let Some((dialog, stop_index)) = goods_dialog_stop() {
                        reprice_dialog_stop(dialog, stop_index, sell, buy);
                    } else if office_open {
                        apply_prices(sell, buy);
                    }
                }
                w if w == CLEAR_ROUTE_KEY => on_clear_route_hotkey(),
                // F1 in the goods dialog: fill the displayed stop's empty slots with
                // buy/sell orders (plain skips the NO_BUY_WARES, ctrl buys everything).
                w if w == SETUP_KEY && goods_dialog_stop().is_some() => {
                    let (dialog, stop_index) = goods_dialog_stop().unwrap();
                    on_dialog_setup_hotkey(dialog, stop_index, !ctrl);
                }
                _ if !office_open => {}
                w if w == SETUP_KEY && ctrl => on_lock_staples_hotkey(),
                w if w == SETUP_KEY && alt => on_lock_building_materials_hotkey(),
                w if w == SETUP_KEY => on_setup_hotkey(),
                _ => {}
            }
        }
    }
    CallNextHookEx(HHOOK::default(), code, wparam, lparam)
}

/// The town whose view is open, from the town-scene object. On the world map this is the
/// town the player last visited.
unsafe fn current_town_index() -> Option<u8> {
    let scene = *TOWN_SCENE_PTR;
    if scene == 0 {
        return None;
    }
    Some(*((scene + TOWN_SCENE_CURRENT_TOWN_OFFSET) as *const u32) as u8)
}

/// The "Automatic maritime trading" (goods) dialog object, constructed at startup
/// (ctor 0x403020, vtable 0x66a7f0, allocated by the mass-constructor at 0x424eb0).
/// Its +0xa4 holds the POOL INDEX of the displayed stop - its arrows walk the pool
/// chain through it - and the close method (vtable+0x118 = 0x4066f0) sets it to -1,
/// so one field carries both "is open" and "which stop".
const DIALOG_PTR: *const u32 = 0x006cba74 as _;
const DIALOG_STOP_INDEX_OFFSET: u32 = 0xa4;
const DIALOG_SHIP_INDEX_OFFSET: u32 = 0xa8;
const DIALOG_FLAG_OFFSET: u32 = 0x547c;
/// thiscall(this, stop_pool_index, ship_index, flag): populate the dialog's widgets
/// (direction arrows, amount and price texts) from the stop record. Called by the
/// dialog's own arrows and by the panel's Goods button (0x48c432).
const DIALOG_POPULATE: u32 = 0x00405a20;

/// The goods dialog and the pool index of the stop it displays, if it is open.
unsafe fn goods_dialog_stop() -> Option<(u32, u32)> {
    let dialog = *DIALOG_PTR;
    if dialog == 0 {
        return None;
    }
    let stop_index = *((dialog + DIALOG_STOP_INDEX_OFFSET) as *const i32);
    if stop_index < 0 || stop_index as u16 >= *ROUTE_STOP_POOL_COUNT {
        return None;
    }
    Some((dialog, stop_index as u32))
}

/// Repopulate the goods dialog from its stop record, the way its own arrows do. The
/// displayed texts are sprintf-cached in the dialog object, so in-place pool writes
/// stay invisible without this.
unsafe fn refresh_goods_dialog(dialog: u32, stop_index: u32) {
    let populate: extern "thiscall" fn(u32, u32, u32, u32) = mem::transmute(DIALOG_POPULATE);
    let ship_index = *((dialog + DIALOG_SHIP_INDEX_OFFSET) as *const u32);
    let flag = *((dialog + DIALOG_FLAG_OFFSET) as *const u8) as u32;
    populate(dialog, stop_index, ship_index, flag);
}

/// F9 (THROWAWAY): install an operation logger on the queue drain's call into the
/// operation switch (execute_operations 0x546870 calls 0x535760 at 0x546934), dumping
/// every processed operation. Used to identify the opcode behind UI actions - press F9
/// once, perform the action in-game, read the log.
static OP_LOGGER_HOOK: AtomicPtr<CallRel32Hook> = AtomicPtr::new(std::ptr::null_mut());
const OP_SWITCH_DRAIN_CALL_OFFSET: u32 = 0x146934;
/// Noisy periodic opcodes to omit (0x94/0x24/0x7b per the gitbook's debugging notes).
const OP_LOGGER_NOISE: [u32; 3] = [0x94, 0x24, 0x7b];

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

/// F9 (THROWAWAY): find what marks a town as enterable while the player's ship is
/// arriving but has not docked yet. Press F9 once while the ship is still at sea (takes
/// a byte-exact snapshot of every town struct and dumps the ship's movement state),
/// then again the moment the town's tavern becomes reachable: the second press diffs
/// every town against the snapshot and dumps the ships again. Keep the two presses
/// close together - a day boundary in between adds price and stock noise to the diff.
static TOWN_SNAPSHOT: Mutex<Vec<u8>> = Mutex::new(Vec::new());

#[allow(dead_code)]
unsafe fn on_town_snapshot_hotkey() {
    if TOWN_SNAPSHOT.lock().unwrap().is_empty() {
        let towns = snapshot_towns();
        dump_player_ships("before");
        info!("town probe: snapshot of {towns} towns taken - press F9 again when the town becomes enterable");
    } else {
        diff_towns();
        dump_player_ships("after");
        info!("town probe: diff done, snapshot cleared");
    }
}

/// The side-room missions of the save this probe was written against, counted per town
/// index (Edinburgh 0 .. Novgorod 23): Edinburgh pirate hunter, London trader, Hamburg
/// trader, Rostock trader + courier, Oslo patrol + smuggler + courier, Malmö smuggler,
/// Gdansk escort, Reval fugitive + treasure map, Ladoga patrol.
const MISSIONS_PER_TOWN: [u8; 24] = [1, 0, 1, 0, 0, 0, 0, 0, 1, 0, 2, 0, 3, 0, 1, 0, 0, 0, 1, 0, 0, 2, 1, 0];

/// F9 (THROWAWAY): dump the inline symbol strings the UI's rich-text markup splices in.
///
/// F9 (THROWAWAY): dump the tick pacer's speed block, from the sea-battle speed work
/// (now the gitbook's basics/time.md Game Speed section).
///
/// The pacer (`0x00546640`) turns elapsed real ms (`[0x6DCCF8]` minus `ops+0x938`)
/// into an advance-time operation (opcode 0xC4) sized by the pacing mode `ops+0x92C`:
/// mode 0 = normal play, one tick per `ops+0x8D4` ms (the speed slider's divisor,
/// cap 8/batch); 1 = fast forward, per `ops+0x8D8` (cap 256); 2 = local map, per the
/// constant `[0x673CF8]` = 3375 (cap 1). `ops+0x914` is the master run flag.
#[allow(dead_code)]
unsafe fn on_speed_probe_hotkey() {
    let ops = 0x006df2f0u32;
    let r = |off: u32| *((ops + off) as *const u32);
    debug!(
        "speed probe: level {} run {} advancing {} net {} | div1 {} div2 {} saved_div {} f91c {} | last_ms {} pending_c4 {} timer {} interval {} | const_ms {} clock_ms {} tick {:#x} queue {}",
        r(0x92c),
        r(0x914),
        r(0x918),
        r(0x928),
        r(0x8d4),
        r(0x8d8),
        r(0x8dc),
        r(0x91c),
        r(0x938),
        r(0x93c),
        r(0x940),
        r(0x944),
        *(0x673cf8u32 as *const u32),
        *(0x6dccf8u32 as *const u32),
        *(0x6de4b4u32 as *const u32),
        *((ops + 0x482) as *const u16),
    );
}

/// F10 (THROWAWAY): toggle "battle time follows the speed slider".
///
/// A sea battle switches the tick pacer to level 2, whose branch ignores the slider
/// and paces the world at the hard constant `[0x673CF8]` = 3375 ms per tick
/// (`0x0054675C: mov edi,[0x673CF8]`). The patch replaces that one load - same
/// length, in place - with `mov edi,[esi+0x8D4]`, the level-0 branch's own divisor,
/// so battle time runs at whatever the slider was set to (3515 slowdown .. 78 very
/// fast). The level-2 cap of 1 tick per pacer run stays: at 60 fps that allows ~60
/// ticks/s, far above very fast's 12.8, so it never binds. (An earlier probe that
/// enqueued the fast-forward op 0xC8 level 1 instead flickered the world view over
/// the battle scene - level 1 is the fast-forward MODE with its own window, not a
/// speed.)
/// (retired F9) The layout routine at `0x00462520` expands `\\C`, `\\L` and `\\B` by taking the
/// `char*` at `+0x4` of the objects in `0x006CC37C`, `0x006CC384` and `0x006CC380`
/// (`0x004627F5`..`0x0046281F`), so they are text, not graphics - and can be appended to
/// any string drawn the ordinary way. This prints them, and their neighbours in that
/// global cluster, as bytes and as characters.
#[allow(dead_code)]
unsafe fn on_current_town_hotkey() {
    for global in (0x006cc370..=0x006cc394u32).step_by(4) {
        let object = *(global as *const u32);
        if !(0x0001_0000..0x7fff_0000).contains(&object) {
            debug!("{global:#010x}: {object:#010x} (not a pointer)");
            continue;
        }
        let text = *((object + 4) as *const u32);
        if !(0x0001_0000..0x7fff_0000).contains(&text) {
            debug!("{global:#010x}: object {object:#010x}, +4 = {text:#010x} (not a pointer)");
            continue;
        }
        let mut bytes = Vec::new();
        for offset in 0..32u32 {
            let byte = *((text + offset) as *const u8);
            if byte == 0 {
                break;
            }
            bytes.push(byte);
        }
        let hex: Vec<String> = bytes.iter().map(|b| format!("{b:02x}")).collect();
        let shown: String = bytes
            .iter()
            .map(|b| if (0x20..0x7f).contains(b) { *b as char } else { '.' })
            .collect();
        debug!("{global:#010x}: object {object:#010x} -> \"{shown}\" [{}]", hex.join(" "));
    }
}

/// F9 (THROWAWAY, kept): scan every committed page for a per-town table matching
/// `MISSIONS_PER_TOWN` - the fallback for when a link is in neither the town struct nor
/// the records.
#[allow(dead_code)]
unsafe fn scan_memory_for_mission_table() {
    let towns = GAME_WORLD_PTR.get_towns_count().min(0xff) as usize;
    if towns != MISSIONS_PER_TOWN.len() {
        error!("mission probe: this save has {towns} towns, the expectation has {}", MISSIONS_PER_TOWN.len());
        return;
    }

    let mut region = MEMORY_BASIC_INFORMATION::default();
    let mut address: usize = 0x10000;
    let mut scanned = 0usize;
    let mut hits = 0usize;
    while address < 0x7fff_0000 {
        if VirtualQuery(Some(address as _), &mut region, mem::size_of::<MEMORY_BASIC_INFORMATION>()) == 0 {
            break;
        }
        let base = region.BaseAddress as usize;
        let size = region.RegionSize;
        address = base + size.max(0x1000);

        // Committed, readable, not a guard page - anything else faults on read.
        const READABLE: u32 = 0x02 | 0x04 | 0x08 | 0x20 | 0x40 | 0x80;
        if region.State != MEM_COMMIT || region.Protect.0 & READABLE == 0 || region.Protect.0 & 0x100 != 0 {
            continue;
        }
        scanned += size;
        hits += scan_region_for_mission_table(base, size, towns);
    }

    info!("mission probe: scanned {} MiB, {hits} hits", scanned / (1024 * 1024));
}

/// The shapes a per-town mission table could have, tested at every offset of a region.
/// Each has a cheap first test (a town without a mission) before the full comparison.
unsafe fn scan_region_for_mission_table(base: usize, size: usize, towns: usize) -> usize {
    let has_mission = |town: usize| MISSIONS_PER_TOWN[town] > 0;
    let mut hits = 0;
    let stride_end = size.saturating_sub(4 * towns);
    for offset in 0..stride_end {
        let at = base + offset;

        // One byte per town: the mission count, or any nonzero marker where a mission is.
        let first: u8 = ptr::read_unaligned(at as *const u8);
        if first != 0 && ptr::read_unaligned((at + 1) as *const u8) == 0 {
            let bytes: Vec<u8> = (0..towns).map(|t| ptr::read_unaligned((at + t) as *const u8)).collect();
            if (0..towns).all(|t| bytes[t] == MISSIONS_PER_TOWN[t]) {
                debug!("mission COUNTS (u8) at {at:#010x}{}: {bytes:?}", describe_address(at));
                hits += 1;
            } else if (0..towns).all(|t| (bytes[t] != 0) == has_mission(t)) {
                debug!("u8 set where a mission is at {at:#010x}{}: {bytes:?}", describe_address(at));
                hits += 1;
            } else if (0..towns).all(|t| (bytes[t] != 0xff) == has_mission(t)) {
                debug!("u8 0xff-empty at {at:#010x}{}: {bytes:?}", describe_address(at));
                hits += 1;
            }
        }

        // One id per town, the usual 0xFFFF or 0 for "no mission here".
        for empty in [0u16, 0xffff] {
            if ptr::read_unaligned((at + 2) as *const u16) == empty && ptr::read_unaligned(at as *const u16) != empty {
                let words: Vec<u16> = (0..towns).map(|t| ptr::read_unaligned((at + 2 * t) as *const u16)).collect();
                if (0..towns).all(|t| (words[t] != empty) == has_mission(t)) {
                    debug!("u16 ids ({empty:#06x} = none) at {at:#010x}{}: {words:?}", describe_address(at));
                    hits += 1;
                }
            }
        }

        // Same, as dwords - an id, a pointer or a count per town.
        for empty in [0u32, 0xffff_ffff] {
            if ptr::read_unaligned((at + 4) as *const u32) == empty && ptr::read_unaligned(at as *const u32) != empty {
                let dwords: Vec<u32> = (0..towns).map(|t| ptr::read_unaligned((at + 4 * t) as *const u32)).collect();
                if (0..towns).all(|t| (dwords[t] != empty) == has_mission(t)) {
                    debug!("u32 ids ({empty:#010x} = none) at {at:#010x}{}: {dwords:x?}", describe_address(at));
                    hits += 1;
                }
            }
        }
    }
    hits
}

/// Where an address sits, so a hit can be placed: inside a town struct (with the town and
/// the field offset), in the executable's own data, or in unidentified heap.
unsafe fn describe_address(at: usize) -> String {
    let towns_base: u32 = GAME_WORLD_PTR.get(0x68);
    let towns_end = towns_base as usize + GAME_WORLD_PTR.get_towns_count() as usize * TOWN_SIZE as usize;
    if (towns_base as usize..towns_end).contains(&at) {
        let delta = at - towns_base as usize;
        return format!(" (town {} +{:#x})", delta / TOWN_SIZE as usize, delta % TOWN_SIZE as usize);
    }
    if (0x0040_0000..0x0080_0000).contains(&at) {
        return " (executable data)".into();
    }
    let pool_base: u32 = SCHEDULED_TASKS_PTR.get(0x0);
    let capacity: u16 = SCHEDULED_TASKS_PTR.get(0x0c);
    let stride = SCHEDULED_TASK_SIZE as usize;
    let pool_end = pool_base as usize + capacity as usize * stride;
    if (pool_base as usize..pool_end).contains(&at) {
        let delta = at - pool_base as usize;
        return format!(" (scheduled task {} +{:#x})", delta / stride, delta % stride);
    }
    String::new()
}

/// Every town struct, byte for byte, concatenated.
unsafe fn snapshot_towns() -> u8 {
    let count = GAME_WORLD_PTR.get_towns_count() as u8;
    let mut buffer = Vec::with_capacity(count as usize * TOWN_SIZE as usize);
    for town_index in 0..count {
        let town = GAME_WORLD_PTR.get_town(town_index);
        buffer.extend_from_slice(std::slice::from_raw_parts(town.get_address() as *const u8, TOWN_SIZE as usize));
    }
    *TOWN_SNAPSHOT.lock().unwrap() = buffer;
    count
}

/// Log every run of bytes that changed since the snapshot, per town.
unsafe fn diff_towns() {
    let snapshot = std::mem::take(&mut *TOWN_SNAPSHOT.lock().unwrap());
    let size = TOWN_SIZE as usize;
    for town_index in 0..(snapshot.len() / size) as u8 {
        let town = GAME_WORLD_PTR.get_town(town_index);
        let now = std::slice::from_raw_parts(town.get_address() as *const u8, size);
        let before = &snapshot[town_index as usize * size..][..size];
        let name = get_town_name(town_index).unwrap_or_else(|| "<unknown>".into());

        let mut offset = 0;
        while offset < size {
            if now[offset] == before[offset] {
                offset += 1;
                continue;
            }
            let start = offset;
            while offset < size && now[offset] != before[offset] {
                offset += 1;
            }
            let old: Vec<String> = before[start..offset].iter().map(|b| format!("{b:02x}")).collect();
            let new: Vec<String> = now[start..offset].iter().map(|b| format!("{b:02x}")).collect();
            debug!("town diff {name} +{start:#x}: {} -> {}", old.join(" "), new.join(" "));
        }
    }
}

/// The movement state of every ship the player owns: the fields that could plausibly
/// carry "arriving at a town" - status (+0x134), the flag bytes around the destination
/// (+0x3C..+0x3E), the counter at +0x138, and the owning convoy's status and town.
unsafe fn dump_player_ships(label: &str) {
    let ships = p3_api::ships::ShipsPtr::new();
    let player_merchant = OPERATIONS_PTR.get_player_merchant_index() as u8;
    for index in 0..ships.get_ships_size() {
        let Some(ship) = ships.get_ship(index) else { break };
        if ship.get_merchant_index() != player_merchant {
            continue;
        }
        let convoy_id = ship.get_convoy_id();
        let (convoy_status, convoy_town) = match ships.get_convoy(convoy_id) {
            Some(convoy) => (convoy.get_status() as i32, convoy.get_current_town_index() as i32),
            None => (-1, -1),
        };
        let flags: [u8; 3] = [ship.get(0x3c), ship.get(0x3d), ship.get(0x3e)];
        let counter: u16 = ship.get(0x138);
        debug!(
            "{label}: ship {index} {} status {:#x} dest {:?} last {:?} flags {:02x}/{:02x}/{:02x} +0x138 {counter} convoy {convoy_id} (status {convoy_status:#x} town {convoy_town}) pos {},{}",
            ship.get_name(),
            ship.get_status(),
            ship.get_destination_town_index(),
            ship.get_last_town_index(),
            flags[0],
            flags[1],
            flags[2],
            ship.get_x() >> 16,
            ship.get_y() >> 16
        );
    }
}

/// Kept from the previous investigation (the auto-trader chain work): the op logger
/// installer and the tavern-captain census.
#[allow(dead_code)]
unsafe fn install_op_logger() {
    if !OP_LOGGER_HOOK.load(Ordering::SeqCst).is_null() {
        return;
    }
    match hook_call_rel32(OP_SWITCH_DRAIN_CALL_OFFSET, op_logger_hook as usize as u32) {
        Ok(hook) => {
            OP_LOGGER_HOOK.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst);
            info!("op logger: installed - perform the action to identify");
        }
        Err(e) => error!("op logger: hook failed: {e:?}"),
    }
}

/// F9 (THROWAWAY): dump the name-registry neighborhood past bank C (pointers at
/// 0x6DDB48.., counts 0x6DDB70/0x6DDB74) to identify the first/last-name tables the
/// auto-trader name ids index.
#[allow(dead_code)]
unsafe fn dump_name_registry() {
    let first_count = *(0x006ddb70 as *const u16);
    let last_count = *(0x006ddb74 as *const u16);
    debug!("name registry counts: first {first_count}, last {last_count}");
    for i in 0..10u32 {
        let slot = 0x006ddb48 + i * 4;
        let ptr = *(slot as *const u32);
        if !(0x0001_0000..0x7fff_0000).contains(&ptr) {
            debug!("name table slot {slot:#010x}: {ptr:#010x} (not a pointer)");
            continue;
        }
        let mut preview = String::new();
        for off in 0..96u32 {
            let b = *((ptr + off) as *const u8);
            preview.push(if b == 0 {
                '|'
            } else if (0x20..0x7f).contains(&b) {
                b as char
            } else {
                '.'
            });
        }
        debug!("name table slot {slot:#010x} -> {ptr:#010x}: {preview}");
    }
}

/// F9 (THROWAWAY): list the towns whose tavern has a hireable captain, replicating
/// the game's resolver 0x5261d0: walk the town's auto-trader chain, a hireable
/// captain is an available (state > 0x20) unemployed (merchant 0xff) record. Logs
/// every chain record to the debug log for offset verification.
#[allow(dead_code)]
unsafe fn find_tavern_captains() {
    debug!("today's date serial: {}", *(0x006de4b4 as *const u32));
    dump_name_registry();
    let ships = p3_api::ships::ShipsPtr::new();
    let count = ships.get_auto_traders_size();
    let mut chained: Vec<(u16, String)> = Vec::new();
    let mut found = 0;
    for town_index in 0..GAME_WORLD_PTR.get_towns_count() as u8 {
        let town = get_town_name(town_index).unwrap_or_else(|| "<unknown>".into());
        let mut index = GAME_WORLD_PTR.get_town(town_index).get_auto_trader_chain_head();
        // The chain ends on an out-of-range index; cap the walk against cycles.
        for _ in 0..count {
            let Some(trader) = ships.get_auto_trader(index) else { break };
            chained.push((index, town.clone()));
            if trader.is_captain() && trader.get_merchant_index() == 0xff {
                found += 1;
                notify(&format!(
                    "Captain for hire in {town}: nav {} trade {} combat {} (43/level), wage {}",
                    trader.get_navigation_skill(),
                    trader.get_trade_skill(),
                    trader.get_combat_skill(),
                    trader.get_daily_wage()
                ));
            }
            index = trader.get_next_index();
        }
    }
    // The whole array: chained records tagged with their town, the rest "unchained" -
    // employed captains, parked administrators, free slots.
    for index in 0..count {
        let Some(trader) = ships.get_auto_trader(index) else { break };
        let location = chained
            .iter()
            .find(|(i, _)| *i == index)
            .map(|(_, town)| town.clone())
            .unwrap_or_else(|| "unchained".into());
        let share = if trader.is_pirate() {
            format!(" share {}%", trader.get_pirate_loot_share_percent())
        } else {
            String::new()
        };
        debug!(
            "auto trader {index} ({location}): state {:#04x}{share} names {}/{} field4 {} nav {} trade {} combat {} wage {} merchant {:#04x} next {:#x}",
            trader.get_state_byte(),
            trader.get_first_name_id(),
            trader.get_last_name_id(),
            trader.get_timestamp(),
            trader.get_navigation_skill(),
            trader.get_trade_skill(),
            trader.get_combat_skill(),
            trader.get_daily_wage(),
            trader.get_merchant_index(),
            trader.get_next_index()
        );
    }
    if found == 0 {
        notify("No captain is waiting in any tavern");
    }
    // Every ship with a captain: correlates the unchained auto-trader records with
    // the ships employing them (ship+0x42, incl. town-owned and pirate ships).
    for ship_index in 0..ships.get_ships_size() {
        let Some(ship) = ships.get_ship(ship_index) else { continue };
        let captain = ship.get_captain_index();
        if captain < count {
            debug!(
                "ship {ship_index} {:?} (owner {:#04x}): captain {captain}",
                ship.get_name(),
                ship.get_merchant_index()
            );
        }
    }
}

/// The ship currently shown in the ship panel (the map selection).
unsafe fn selected_ship_index() -> Option<u16> {
    UIShipPanelPtr::new().get_selected_ship_index()
}

/// True if `ship_index` belongs to the player (walk the player merchant's ship chain).
unsafe fn is_player_ship(ship_index: u16) -> bool {
    let ships = p3_api::ships::ShipsPtr::new();
    let ships_size = ships.get_ships_size();
    let merchant = GAME_WORLD_PTR.get_merchant(OPERATIONS_PTR.get_player_merchant_index() as u16);
    let mut idx = merchant.get_first_ship_index();
    for _ in 0..2000 {
        if idx >= ships_size {
            break;
        }
        if idx == ship_index {
            return true;
        }
        let Some(ship) = ships.get_ship(idx) else { break };
        idx = ship.get_next_ship_index_of_merchant();
    }
    false
}

/// Read a 220-byte pool record into a TradeRouteStop (the file layout, head field dropped).
unsafe fn read_pool_stop(record: u32) -> TradeRouteStop {
    let mut order = [0u8; 24];
    core::ptr::copy_nonoverlapping((record + 4) as *const u8, order.as_mut_ptr(), 24);
    let mut price = [0i32; 24];
    let mut amount = [0i32; 24];
    for i in 0..24u32 {
        price[i as usize] = *((record + 28 + i * 4) as *const i32);
        amount[i as usize] = *((record + 124 + i * 4) as *const i32);
    }
    TradeRouteStop {
        town_index: *((record + 2) as *const u8),
        action: *((record + 3) as *const u8),
        order,
        price,
        amount,
    }
}

/// Read a ship's current route by walking the pool chain, rotated so the logical first
/// stop (the one carrying the 0x04 marker) is first. Empty if the ship has no route.
/// The pool record addresses of a ship's route, rotated so the logical first stop (the
/// one carrying the 0x04 marker) is first - the order the route window displays.
unsafe fn route_stop_records(ship_index: u16) -> Vec<u32> {
    let ships = p3_api::ships::ShipsPtr::new();
    let Some(ship) = ships.get_ship(ship_index) else {
        return Vec::new();
    };
    let pool = *ROUTE_STOP_POOL;
    let pool_count = *ROUTE_STOP_POOL_COUNT;
    let head = *((ship.address + SHIP_ROUTE_HEAD_OFFSET) as *const u16);
    if pool == 0 || head >= pool_count {
        return Vec::new();
    }
    let mut records = Vec::new();
    let mut idx = head;
    for _ in 0..64 {
        let record = pool + idx as u32 * ROUTE_STOP_SIZE;
        records.push(record);
        let next = *(record as *const u16);
        if next == idx || next >= pool_count {
            break;
        }
        idx = next;
        if idx == head {
            break;
        }
    }
    if let Some(pos) = records.iter().position(|&r| *((r + 3) as *const u8) & FIRST_STOP_MARKER != 0) {
        records.rotate_left(pos);
    }
    records
}

unsafe fn read_ship_route(ship_index: u16) -> Vec<TradeRouteStop> {
    route_stop_records(ship_index).into_iter().map(|record| read_pool_stop(record)).collect()
}

/// Save the ship's current route to [ROUTE_BACKUP_PATH], loadable from the route
/// window's File button as "_backup". Returns the stops for reuse.
unsafe fn backup_route(ship_index: u16) -> Vec<TradeRouteStop> {
    let previous = read_ship_route(ship_index);
    if !previous.is_empty() {
        let count = previous.len();
        let backup = p3_rou::TradeRouteFile { stops: previous.clone() }.serialize();
        match std::fs::write(ROUTE_BACKUP_PATH, &backup) {
            Ok(()) => info!("route: saved the previous {count} stops to {ROUTE_BACKUP_PATH}"),
            Err(e) => error!("route: failed to back up the previous route: {e}"),
        }
    }
    previous
}

/// DEL: clear the selected ship's route by enqueueing the game's own stop-removal
/// operation for every stop - the op the route panel's town "none" selection sends,
/// identifying each stop by its POOL INDEX (stable across the removals, unlike
/// positions). Guarded on the goods dialog being closed: it displays a stop of this
/// route, and the removals free the pool records it points into.
unsafe fn on_clear_route_hotkey() {
    if goods_dialog_stop().is_some() {
        notify("Clear route: close the goods dialog first");
        return;
    }
    let Some(ship_index) = selected_ship_index() else {
        notify("Clear route: no ship selected");
        return;
    };
    if !is_player_ship(ship_index) {
        notify("Clear route: the selected ship is not yours");
        return;
    }
    let previous = backup_route(ship_index);
    if previous.is_empty() {
        notify("Clear route: the ship has no route");
        return;
    }
    // Deactivate first - the removals do not touch the active flag - then remove every
    // stop, the way transfer_loaded_traderoute also deactivates before rebuilding.
    OPERATIONS_PTR.enqueue_operation(Operation::SetTradeRouteActive {
        ship_index: ship_index as u32,
        active: false,
    });
    let pool = *ROUTE_STOP_POOL;
    for record in route_stop_records(ship_index) {
        OPERATIONS_PTR.enqueue_operation(Operation::RemoveTradeRouteStop {
            stop_pool_index: (record - pool) / ROUTE_STOP_SIZE,
            ship_index: ship_index as u32,
        });
    }
    let ships = p3_api::ships::ShipsPtr::new();
    let name = ships.get_ship(ship_index).map(|s| s.get_name()).unwrap_or_default();
    notify(&format!("Route cleared: {} stops removed from {name}", previous.len()));
}

/// Ctrl/Alt+QWERTY while the goods dialog is open: reprice the stop being edited, in
/// place. The dialog edits the applied route's pool record directly (verified: its +/-
/// buttons change the pool), so we write the level prices into the same record: sells
/// (positive price) get the sell level, buys (negated max price) the buy level.
/// Directions and amounts stay untouched.
unsafe fn reprice_dialog_stop(dialog: u32, stop_index: u32, sell: Option<PriceLevel>, buy: Option<PriceLevel>) {
    let record = *ROUTE_STOP_POOL + stop_index * ROUTE_STOP_SIZE;
    let town = get_town_name(*((record + 2) as *const u8)).unwrap_or_else(|| "<unknown>".into());

    let mut updated = 0;
    for ware_index in TRADE_WARES {
        // Only slots with a nonzero amount carry an instruction: inactive slots can
        // still hold a (positive) base price the dialog has not normalized yet.
        let amount = *((record + 124 + ware_index as u32 * 4) as *const i32);
        if amount == 0 {
            continue;
        }
        let price = (record + 28 + ware_index as u32 * 4) as *mut i32;
        let new = match *price {
            0 => continue,
            p if p < 0 => match buy {
                Some(level) => -buy_price(ware_index, level),
                None => continue,
            },
            _ => match sell {
                Some(level) => sell_price(ware_index, level),
                None => continue,
            },
        };
        if new != *price {
            *price = new;
            updated += 1;
        }
    }
    if updated > 0 {
        refresh_goods_dialog(dialog, stop_index);
    }
    let what = match (sell, buy) {
        (Some(level), _) => format!("Sell prices {level:?}"),
        (_, Some(level)) => format!("Buy prices {level:?}"),
        _ => "Prices".into(),
    };
    notify(&format!("{what}: {updated} wares of the {town} stop"));
}

/// F1 while the goods dialog is open: fill the displayed stop's EMPTY ware slots with
/// trade orders for the stop's own town - buy what it produces (at STOP_BUY_LEVEL),
/// sell everything else (at STOP_SELL_LEVEL), MAX amounts - exactly F4's stop, but in
/// place and preserving every existing instruction, office transfers included. With
/// `skip_no_buy_wares` the [NO_BUY_WARES] get no order where the town produces them.
/// The instruction order is recomputed into the builder's cargo order.
unsafe fn on_dialog_setup_hotkey(dialog: u32, stop_index: u32, skip_no_buy_wares: bool) {
    let record = *ROUTE_STOP_POOL + stop_index * ROUTE_STOP_SIZE;
    let town_index = *((record + 2) as *const u8);
    let town = get_town_name(town_index).unwrap_or_else(|| "<unknown>".into());
    let production = GAME_WORLD_PTR.get_town(town_index).get_production_values();

    let mut price = [0i32; 24];
    let mut amount = [0i32; 24];
    core::ptr::copy_nonoverlapping((record + 28) as *const i32, price.as_mut_ptr(), 24);
    core::ptr::copy_nonoverlapping((record + 124) as *const i32, amount.as_mut_ptr(), 24);

    let mut bought = Vec::new();
    let mut skipped = Vec::new();
    let mut sold = 0;
    let mut untouched = 0;
    for ware_index in TRADE_WARES {
        let i = ware_index as usize;
        let ware_id = WareId::from_u16(ware_index).unwrap();
        // A slot without an instruction has amount 0 - but NOT necessarily price 0:
        // the game leaves base prices in inactive slots, and the dialog normalizes
        // them to 0 lazily via enqueued operations after opening. Testing the price
        // here would race that queue (observed: quick F1 presses filled fewer wares).
        if amount[i] != 0 {
            untouched += 1;
            continue;
        }
        if production[i] > 0 {
            if skip_no_buy_wares && NO_BUY_WARES.contains(&ware_id) {
                skipped.push(format!("{ware_id:?}"));
                continue;
            }
            price[i] = -buy_price(ware_index, STOP_BUY_LEVEL);
            bought.push(format!("{ware_id:?}"));
        } else {
            price[i] = sell_price(ware_index, STOP_SELL_LEVEL);
            sold += 1;
        }
        amount[i] = builder::MAX_AMOUNT;
    }

    core::ptr::copy_nonoverlapping(price.as_ptr(), (record + 28) as *mut i32, 24);
    core::ptr::copy_nonoverlapping(amount.as_ptr(), (record + 124) as *mut i32, 24);
    let order = builder::cargo_order(&amount);
    core::ptr::copy_nonoverlapping(order.as_ptr(), (record + 4) as *mut u8, 24);
    refresh_goods_dialog(dialog, stop_index);

    let skipped = if skipped.is_empty() {
        String::new()
    } else {
        format!(", skipping [{}]", skipped.join(", "))
    };
    info!(
        "dialog setup for the {town} stop: buying [{}]{skipped}, selling {sold} others, {untouched} existing instructions untouched",
        bought.join(", ")
    );
    notify(&format!(
        "Stop setup for {town}: {} buys, {sold} sells, {untouched} kept",
        bought.len()
    ));
}

/// Apply a route file (written to ROUTE_FILE_PATH) to a ship, mimicking the game's own
/// route Load: build the base-name string object, set the target ship, call the loader,
/// hand the resulting buffer to transfer (which frees it).
unsafe fn apply_route_file(ship_index: u16) -> bool {
    let mut name: Vec<u8> = ROUTE_BASE_NAME.bytes().collect();
    name.push(0);

    // Start from the empty-string sentinel, like a default-constructed object, so the
    // constructor's "release the old data" path is a no-op instead of a wild free.
    let mut string_object: u32 = *STRING_EMPTY_HEADER_PTR + 0xc;
    let ctor: extern "thiscall" fn(*mut u32, *const u8) -> *mut u32 = mem::transmute(STRING_CTOR_FROM_CSTR);
    ctor(&mut string_object, name.as_ptr());
    debug!("add stop: name string built");

    *OPERATIONS_ROUTE_SHIP = ship_index as u32;
    let loader: extern "thiscall" fn(u32, *const u32) -> u32 = mem::transmute(ROUTE_LOADER_ADDRESS);
    let buffer = loader(ROUTE_LOADER_THIS, &string_object);
    debug!("add stop: loader returned buffer {buffer:#010x}");

    let dtor: extern "thiscall" fn(*mut u32) = mem::transmute(STRING_DTOR);
    dtor(&mut string_object);

    if buffer == 0 {
        return false;
    }
    *OPERATIONS_ROUTE_BUFFER = buffer;
    OPERATIONS_PTR.transfer_loaded_traderoute();
    true
}

/// The context every route key needs: the selected ship (which must be the player's) and
/// the town whose view is open. Logs why not when it cannot be resolved.
unsafe fn route_context(what: &str) -> Option<(u16, String, u8)> {
    let Some(ship_index) = selected_ship_index() else {
        notify(&format!("{what}: no ship selected"));
        return None;
    };
    if !is_player_ship(ship_index) {
        notify(&format!("{what}: the selected ship is not yours"));
        return None;
    }
    let Some(town_index) = current_town_index() else {
        notify(&format!("{what}: not in a town"));
        return None;
    };
    let ships = p3_api::ships::ShipsPtr::new();
    let ship_name = ships.get_ship(ship_index).map(|s| s.get_name()).unwrap_or_default();
    Some((ship_index, ship_name, town_index))
}

/// True if the player has a trading office in the town. Route stops that transfer wares
/// to or from an office are wiped at load time in towns where there is none.
unsafe fn has_player_office(town_index: u8) -> bool {
    let merchant_index = OPERATIONS_PTR.get_player_merchant_index();
    GAME_WORLD_PTR.get_office_in_of(town_index as _, merchant_index as _).is_some()
}

/// Write the stops as the route file and apply them to the ship, normalising the
/// logical-first marker onto the first stop. Returns false and logs on failure.
unsafe fn write_and_apply_route(ship_index: u16, mut stops: Vec<TradeRouteStop>) -> bool {
    for (i, stop) in stops.iter_mut().enumerate() {
        if i == 0 {
            stop.action |= FIRST_STOP_MARKER;
        } else {
            stop.action &= !FIRST_STOP_MARKER;
        }
    }
    let data = p3_rou::TradeRouteFile { stops }.serialize();
    if let Err(e) = std::fs::write(ROUTE_FILE_PATH, &data) {
        error!("route: failed to write {ROUTE_FILE_PATH}: {e}");
        return false;
    }
    if !apply_route_file(ship_index) {
        error!("route: loader failed (is fix_uncompressed_trade_route_loading.dll installed?)");
        return false;
    }
    true
}

/// Append a trade stop for the current town to the selected ship's route: buy the wares
/// the town produces at the STOP_BUY_LEVEL price, sell all others at the STOP_SELL_LEVEL
/// price, with the sell instructions listed above the buys. With `skip_no_buy_wares` the
/// [NO_BUY_WARES] get no order at all where the town produces them. The whole route is
/// then re-applied through the game's loader + transfer.
unsafe fn on_add_stop_hotkey(skip_no_buy_wares: bool) {
    let Some((ship_index, name, town_index)) = route_context("add stop") else {
        return;
    };
    let town = get_town_name(town_index).unwrap_or_else(|| "<unknown>".into());
    let production = GAME_WORLD_PTR.get_town(town_index).get_production_values();

    let mut price = [0i32; 24];
    let mut amount = [0i32; 24];
    let mut bought = Vec::new();
    let mut skipped = Vec::new();
    let mut sold = 0;
    for ware_index in TRADE_WARES {
        let i = ware_index as usize;
        let ware_id = WareId::from_u16(ware_index).unwrap();
        if production[i] > 0 {
            // Produced here: collect it, up to the buy level's maximum price - unless
            // it is one of the wares we leave in the town.
            if skip_no_buy_wares && NO_BUY_WARES.contains(&ware_id) {
                skipped.push(format!("{ware_id:?}"));
                continue;
            }
            price[i] = -buy_price(ware_index, STOP_BUY_LEVEL);
            bought.push(format!("{ware_id:?}"));
        } else {
            // Not produced here: supply it, down to the sell level's minimum price.
            price[i] = sell_price(ware_index, STOP_SELL_LEVEL);
            sold += 1;
        }
        amount[i] = builder::MAX_AMOUNT;
    }
    // builder::stop orders the instructions: the sells first, then the buys with the
    // barrel goods before the bulky loads goods.
    let mut stops = read_ship_route(ship_index);
    stops.push(builder::stop(town_index, 0x00, price, amount));
    let stop_count = stops.len();

    if write_and_apply_route(ship_index, stops) {
        let skipped = if skipped.is_empty() {
            String::new()
        } else {
            format!(", skipping [{}]", skipped.join(", "))
        };
        info!(
            "add stop in {town} on {name:?}: buying [{}]{skipped}, selling {sold} others (route now {stop_count} stops)",
            bought.join(", ")
        );
        notify(&format!("Stop added in {town}: {name} now {stop_count} stops"));
    }
}

/// The route templates F3 can set, from `p3_rou::builder`.
#[derive(Clone, Copy, Debug)]
enum RouteKind {
    FiveStop,
    SixStop,
    Suck,
}

/// F3: rebuild the selected ship's route from a supply template. The ship's current
/// route provides the towns: its FIRST stop's town becomes the home town, and the
/// remaining unique towns, in order, the targets (further occurrences of the home town
/// are ignored; a ship without a route uses the merchant's home town and the open town
/// view). The generated route repeats the template's action stops once per target,
/// bracketed by a home load stop (summed quantities) and a home unload stop. With
/// shift held, the target is just the currently open town and the generated stops are
/// APPENDED to the existing route instead of replacing it.
///
/// Quantities are a week of each target's citizen and business consumption; prices the
/// R levels. Wares a target produces itself are not supplied to it; targets without a
/// player office get a combined sell-and-buy trade stop (buying their produce at T)
/// instead of the office-reset stops. Ctrl+F3 builds a collection route instead: one
/// buy stop per target at the T prices, skipping the NO_BUY_WARES and everything the
/// home town produces itself.
unsafe fn on_route_hotkey(kind: RouteKind, append: bool) {
    let Some(ship_index) = selected_ship_index() else {
        notify("Route: no ship selected");
        return;
    };
    if !is_player_ship(ship_index) {
        notify("Route: the selected ship is not yours");
        return;
    }
    let ships = p3_api::ships::ShipsPtr::new();
    let name = ships.get_ship(ship_index).map(|s| s.get_name()).unwrap_or_default();
    // Whether replacing or appending, the whole route gets rewritten, so keep the old
    // one loadable from the route window's File button ("_backup") in case this was a
    // mistake. The previous stops also provide the home and target towns.
    let previous = backup_route(ship_index);

    // The route's first stop is the home town; without a route, the merchant's home.
    let load_town = if !append && !previous.is_empty() {
        previous[0].town_index
    } else {
        GAME_WORLD_PTR
            .get_merchant(OPERATIONS_PTR.get_player_merchant_index() as u16)
            .get_hometown_index()
    };
    let load_town_name = get_town_name(load_town).unwrap_or_else(|| "<unknown>".into());

    let targets: Vec<u8> = if append {
        // Shift: append the template for the currently open town.
        let Some(town) = current_town_index() else {
            notify("Route: shift appends for the open town, but no town view is open");
            return;
        };
        vec![town]
    } else {
        let mut towns: Vec<u8> = Vec::new();
        for stop in previous.iter().skip(1) {
            if stop.town_index != load_town && !towns.contains(&stop.town_index) {
                towns.push(stop.town_index);
            }
        }
        // A ship without a route (or one only touching one town) falls back to the
        // open town view, preserving the old single-target workflow.
        if towns.is_empty() {
            match current_town_index() {
                Some(town) if town != load_town => towns.push(town),
                _ => {
                    notify("Route: no target towns in the current route and no town view open");
                    return;
                }
            }
        }
        towns
    };

    // The buy list, shared by the collection route and the office-less trade stops:
    // everything except the NO_BUY_WARES and what the home town produces itself, at
    // the T price. (In trade stops, sells take precedence per ware.)
    let home_production = GAME_WORLD_PTR.get_town(load_town).get_production_values();
    let mut collect_buys = [0i32; 24];
    for ware_index in TRADE_WARES {
        let i = ware_index as usize;
        let ware_id = WareId::from_u16(ware_index).unwrap();
        if home_production[i] <= 0 && !NO_BUY_WARES.contains(&ware_id) {
            collect_buys[i] = buy_price(ware_index, PriceLevel::Lower70);
        }
    }

    let mut total_load = [0i32; 24];
    let mut middle: Vec<TradeRouteStop> = Vec::new();
    let mut described: Vec<String> = Vec::new();
    for &town_index in &targets {
        let town_name = get_town_name(town_index).unwrap_or_else(|| "<unknown>".into());
        if matches!(kind, RouteKind::Suck) {
            middle.push(builder::buy_stop(town_index, &collect_buys));
            described.push(town_name);
            continue;
        }

        let (load_amount, sell_prices, supplied) = town_supply_basket(town_index);
        for i in 0..24 {
            total_load[i] = total_load[i].saturating_add(load_amount[i]);
        }
        if has_player_office(town_index) {
            middle.extend(match kind {
                RouteKind::FiveStop => builder::five_stop_middle(town_index, &load_amount, &sell_prices),
                _ => builder::six_stop_middle(town_index, &load_amount, &sell_prices),
            });
            described.push(format!("{town_name} ({supplied} wares)"));
        } else {
            // No office: the reset stops would be wiped, so trade in one stop - sell
            // the supplies and buy the rest at T through the shared buy filter (no
            // NO_BUY_WARES, nothing the home town produces itself).
            middle.push(builder::trade_stop(town_index, &load_amount, &sell_prices, &collect_buys));
            described.push(format!("{town_name} ({supplied} wares, no office)"));
        }
    }

    let template = builder::bracketed_route(load_town, total_load, middle);
    let stops = if append {
        let mut stops = previous;
        stops.extend(template);
        stops
    } else {
        template
    };
    let stop_count = stops.len();
    let route_name = route_ship_name(&stops);

    if write_and_apply_route(ship_index, stops) {
        rename_ship(ship_index, &route_name);
        let action = if append { "appended to" } else { "set on" };
        let verb = if matches!(kind, RouteKind::Suck) { "collect at T from" } else { "supply" };
        info!(
            "route {kind:?} {action} {name:?}: from {load_town_name}, {verb} [{}] (route now {stop_count} stops)",
            described.join(", ")
        );
        notify(&format!(
            "{kind:?} route {action} {route_name}: {} targets from {load_town_name}, {stop_count} stops",
            described.len()
        ));
    }
}

/// The route name for a ship: the first three letters of each of the route's towns,
/// unique, in route order (e.g. LueRosSte) - up to ten towns. Capped at 31
/// characters: the ship struct's inline name buffer at +0x160 is 32 bytes and ends
/// the 0x180-stride struct, so anything longer would spill into the next ship.
unsafe fn route_ship_name(stops: &[TradeRouteStop]) -> String {
    let mut route_name = String::new();
    let mut seen: Vec<u8> = Vec::new();
    for stop in stops {
        if seen.contains(&stop.town_index) {
            continue;
        }
        seen.push(stop.town_index);
        if route_name.chars().count() + 3 > 31 {
            break;
        }
        let town = get_town_name(stop.town_index).unwrap_or_default();
        route_name.extend(town.chars().take(3));
    }
    route_name
}

/// Rename a ship through the game's rename operations, the way the shipyard does:
/// opcode 0x2d carries the first 12 latin1 bytes, 0x2e chunks append the rest.
unsafe fn rename_ship(ship_index: u16, name: &str) {
    let bytes: Vec<u8> = name.chars().map(|c| if (c as u32) <= 0xff { c as u32 as u8 } else { b'?' }).collect();
    for (i, chunk) in bytes.chunks(12).enumerate() {
        let mut padded = [0u8; 12];
        padded[..chunk.len()].copy_from_slice(chunk);
        let operation = if i == 0 {
            Operation::RenameShip {
                ship_index: ship_index as u32,
                name: padded,
            }
        } else {
            Operation::AppendShipName {
                ship_index: ship_index as u32,
                name: padded,
            }
        };
        OPERATIONS_PTR.enqueue_operation(operation);
    }
}

/// One target town's supply basket: a week of its citizen and business consumption in
/// raw units, rounded up to whole in-game units, with the R sell prices; wares the town
/// produces itself and the NO_SUPPLY_WARES are excluded. Returns (amounts, prices,
/// supplied ware count).
unsafe fn town_supply_basket(town_index: u8) -> ([i32; 24], [i32; 24], u32) {
    let town = GAME_WORLD_PTR.get_town(town_index);
    let citizens = town.get_daily_consumptions_citizens();
    let businesses = town.get_daily_consumptions_businesses();
    let production = town.get_production_values();

    let mut load_amount = [0i32; 24];
    let mut sell_prices = [0i32; 24];
    let mut supplied = 0;
    for ware_index in TRADE_WARES {
        let i = ware_index as usize;
        let ware_id = WareId::from_u16(ware_index).unwrap();
        // A zero load amount is how the route templates express "do not supply this".
        if production[i] > 0 || NO_SUPPLY_WARES.contains(&ware_id) {
            continue;
        }
        // A week of what the town actually consumes - citizens and businesses - in raw
        // units. Not the t0 threshold: that is a comfortable stock level, inflated by
        // minimum floors and construction reserves, so it would have us ferrying goods
        // that never disappear.
        let weekly = (citizens[i] + businesses[i]).saturating_mul(7);
        if weekly == 0 {
            continue; // the town does not consume it
        }
        // Rounded UP to whole in-game units: undersupply empties the office before the
        // ship returns, while the surplus just rides home.
        let scaling = ware_id.get_scaling();
        load_amount[i] = (weekly + scaling - 1) / scaling * scaling;
        sell_prices[i] = sell_price(ware_index, PriceLevel::Center);
        supplied += 1;
    }
    (load_amount, sell_prices, supplied)
}

/// Returns the administrator view's office and its index, or logs why not.
unsafe fn resolve_office() -> Option<(p3_api::data::office::OfficePtr, u16, String)> {
    let window = UITradingOfficeWindowPtr::new();
    if window.get_address() == 0 {
        notify("Office keys: no trading office window open");
        return None;
    }
    let town_index = window.get_town_index();
    let town = get_town_name(town_index as u8).unwrap_or_else(|| "<unknown>".into());

    let page = window.get_selected_page();
    if page != ADMINISTRATOR_PAGE {
        // No popup: F1 doubles as mod-trading-office-details' "all towns" key, so
        // the office keys stay quiet on the other pages.
        debug!("office keys: {town} is on page {page}, not the Trading Office view");
        return None;
    }

    let merchant_index = OPERATIONS_PTR.get_player_merchant_index();
    let Some(office) = GAME_WORLD_PTR.get_office_in_of(town_index as _, merchant_index as _) else {
        notify(&format!("Office keys: no player office found in {town}"));
        return None;
    };
    let office_index = (0..GAME_WORLD_PTR.get_offices_count()).find(|&i| GAME_WORLD_PTR.get_office(i).address == office.address)?;
    Some((office, office_index, town))
}

/// F1: for every ware whose current order is "do nothing", set BUY (at the Center buy
/// level, t1) if the town produces the ware, otherwise SELL (at the Center sell level,
/// t0 - the supply price). Wares that already have an order are left untouched; a buy's
/// amount is set to BUY_AMOUNT only if the current amount is 0.
unsafe fn on_setup_hotkey() {
    let Some((office, office_index, town)) = resolve_office() else {
        return;
    };
    let town_index = UITradingOfficeWindowPtr::new().get_town_index() as u8;
    let production = GAME_WORLD_PTR.get_town(town_index).get_production_values();
    let current_prices = office.get_administrator_trade_prices();
    let stocks = office.get_administrator_trade_stock();

    let mut bought = Vec::new();
    let mut sold = 0;
    let mut untouched = 0;
    for ware_index in TRADE_WARES {
        let ware_id = WareId::from_u16(ware_index).unwrap();
        let i = ware_index as usize;
        if current_prices[i] != 0 {
            untouched += 1;
            continue;
        }
        let (price, stock) = if production[i] > 0 {
            bought.push(format!("{ware_id:?}"));
            let stock = if stocks[i] == 0 {
                BUY_AMOUNT.saturating_mul(ware_id.get_scaling())
            } else {
                stocks[i]
            };
            (-buy_price(ware_index, PriceLevel::Center), stock)
        } else {
            sold += 1;
            (sell_price(ware_index, PriceLevel::Center), stocks[i])
        };
        // Executed directly (not enqueued) so the view refresh below sees the new values.
        execute_operation(&Operation::OfficeAutotradeSettingChange {
            stock,
            price,
            office_index: office_index as _,
            ware_id,
        });
    }
    info!(
        "setup in {town}: buying produced wares [{}], selling {sold} others, {untouched} existing orders untouched",
        bought.join(", ")
    );
    notify(&format!(
        "Office setup in {town}: {} buys, {sold} sells, {untouched} kept",
        bought.len()
    ));
    refresh_administrator_view();
}

/// Raise the administrator amounts of the given wares to the given raw amounts - never
/// lowering one - and lock their quantities (the "Lock min. store quantity for auto
/// trade ships" checkbox), so route ships cannot take the stock. Directions and prices
/// are untouched.
unsafe fn provision_and_lock(what: &str, targets: &[(WareId, i32)]) {
    let Some((office, office_index, town)) = resolve_office() else {
        return;
    };
    let town_index = UITradingOfficeWindowPtr::new().get_town_index() as u8;
    let prices = office.get_administrator_trade_prices();
    let stocks = office.get_administrator_trade_stock();
    let merchant_index = OPERATIONS_PTR.get_player_merchant_index();

    let mut raised = Vec::new();
    for &(ware_id, raw_amount) in targets {
        let i = ware_id as usize;
        if stocks[i] < raw_amount {
            execute_operation(&Operation::OfficeAutotradeSettingChange {
                stock: raw_amount,
                price: prices[i],
                office_index: office_index as _,
                ware_id,
            });
            raised.push(format!("{ware_id:?} {}", raw_amount / ware_id.get_scaling()));
        }
        execute_operation(&Operation::OfficeAutotradeLockChange {
            ware_id,
            town_index: town_index as u16,
            merchant_index: merchant_index as u16,
            lock: true,
        });
    }
    info!("{what} in {town}: locked {} wares, raised [{}]", targets.len(), raised.join(", "));
    notify(&format!("Locked {what} in {town} ({} amounts raised)", raised.len()));
    refresh_administrator_view();
}

/// Ctrl+F1: provision and lock the [LOCKED_STAPLES] at a week of the town's citizen
/// consumption.
unsafe fn on_lock_staples_hotkey() {
    let town_index = UITradingOfficeWindowPtr::new().get_town_index() as u8;
    let citizens = GAME_WORLD_PTR.get_town(town_index).get_daily_consumptions_citizens();
    let targets = LOCKED_STAPLES.map(|ware_id| {
        let scaling = ware_id.get_scaling();
        let weekly = citizens[ware_id as usize].saturating_mul(7);
        (ware_id, (weekly + scaling - 1) / scaling * scaling)
    });
    provision_and_lock("celebration goods", &targets);
}

/// Alt+F1: provision and lock the [LOCKED_BUILDING_MATERIALS].
unsafe fn on_lock_building_materials_hotkey() {
    let targets = LOCKED_BUILDING_MATERIALS.map(|(ware_id, units)| (ware_id, units * ware_id.get_scaling()));
    provision_and_lock("building materials", &targets);
}

/// Sets the price of every ware that has an order, keeping its direction and amount:
/// sells get the sell level's price, buys the buy level's. A None level leaves that
/// direction untouched.
unsafe fn apply_prices(sell: Option<PriceLevel>, buy: Option<PriceLevel>) {
    let Some((office, office_index, town)) = resolve_office() else {
        return;
    };
    let current_prices = office.get_administrator_trade_prices();
    let stocks = office.get_administrator_trade_stock();

    let mut updated = 0;
    for ware_index in TRADE_WARES {
        let i = ware_index as usize;
        let price = match current_prices[i] {
            0 => continue,
            p if p < 0 => match buy {
                Some(level) => -buy_price(ware_index, level),
                None => continue,
            },
            _ => match sell {
                Some(level) => sell_price(ware_index, level),
                None => continue,
            },
        };
        if price == current_prices[i] || price == 0 {
            continue;
        }
        execute_operation(&Operation::OfficeAutotradeSettingChange {
            stock: stocks[i],
            price,
            office_index: office_index as _,
            ware_id: WareId::from_u16(ware_index).unwrap(),
        });
        updated += 1;
    }
    let what = match (sell, buy) {
        (Some(level), _) => format!("Sell prices {level:?}"),
        (_, Some(level)) => format!("Buy prices {level:?}"),
        _ => "Prices".into(),
    };
    notify(&format!("{what}: {updated} wares in {town}"));
    refresh_administrator_view();
}

/// The administrator view's amount rows: widget structs at window+0x9840 + row*0x190,
/// rows in the game's ware display order (the table at 0x698538, sorted at runtime by
/// localized name - the open method's populate loop at 0x5d8f40 reads it the same way).
/// The number widget setter 0x45c930 clamps to [row+0x180]..[row+0x184], stores the
/// value at row+0x188 and refreshes the label text.
const OFFICE_ROW_BASE: u32 = 0x9840;
const OFFICE_ROW_STRIDE: u32 = 0x190;
const WARE_DISPLAY_ORDER: *const u8 = 0x00698538 as _;
const WIDGET_SET_VALUE: u32 = 0x0045c930;

/// Re-selects the administrator page (rebuilds the direction arrows and prices) and
/// pushes the office's amounts into the row widgets the way the window's own open
/// method does - the amounts are populated only there, which is why they never
/// refreshed before. (Cycling the window's close+open repopulates too, but detaches
/// the side menu: the game's real open path goes through a view controller.)
unsafe fn refresh_administrator_view() {
    let window = UITradingOfficeWindowPtr::new();
    if window.get_address() == 0 {
        return;
    }
    window.select_new_page(ADMINISTRATOR_PAGE);

    let merchant_index = OPERATIONS_PTR.get_player_merchant_index();
    let Some(office) = GAME_WORLD_PTR.get_office_in_of(window.get_town_index() as _, merchant_index as _) else {
        return;
    };
    let stocks = office.get_administrator_trade_stock();
    let set_value: extern "thiscall" fn(u32, i32) = mem::transmute(WIDGET_SET_VALUE);
    for row in 0..TRADE_WARES.end as u32 {
        let ware = *WARE_DISPLAY_ORDER.add(row as usize) as usize;
        if ware >= TRADE_WARES.end as usize {
            continue;
        }
        let scaling = WareId::from_usize(ware).unwrap().get_scaling();
        let value = (stocks[ware] / scaling).clamp(0, 9999);
        set_value(window.get_address() + OFFICE_ROW_BASE + row * OFFICE_ROW_STRIDE, value);
    }
}

/// F11: dump thresholds, base prices and all price levels for the open town to the log
/// and to <TownName>.csv in the game directory.
unsafe fn on_town_dump_hotkey() {
    let Some(town_index) = current_town_index() else {
        info!("debug dump: not in a town");
        return;
    };
    let town_name = get_town_name(town_index).unwrap_or_else(|| "<unknown>".into());
    let town = GAME_WORLD_PTR.get_town(town_index);
    let thresholds = town.get_price_thresholds();
    let production = town.get_production_values();
    let citizens = town.get_daily_consumptions_citizens();
    let businesses = town.get_daily_consumptions_businesses();

    debug!(
        "debug dump for {town_name} (live trade difficulty {}):",
        crate::prices::difficulty_d()
    );
    // The price columns follow the hotkeys Q W E R T Y (ascending price), sells (alt)
    // then buys (ctrl).
    let mut csv = String::from(
        "ware,base/unit,\
         sell_Q_t1,sell_W_mid_t0_t1,sell_E_30_t0_t1,sell_R_t0,sell_T_70_0_t0,sell_Y_mid_0_t0,\
         buy_Q_t2,buy_W_mid_t1_t2,buy_E_30_t1_t2,buy_R_t1,buy_T_70_t0_t1,buy_Y_mid_t0_t1,\
         cit/wk,bus/wk,prod/day,(t2-t1)/10,t0 units,t1 units,t2 units,t3 units\n",
    );
    for ware_index in TRADE_WARES {
        let i = ware_index as usize;
        let ware_id = WareId::from_u16(ware_index).unwrap();
        let scaling = ware_id.get_scaling();
        let base_per_unit = crate::prices::base_price_per_unit(ware_index);
        let [t0, t1, t2, t3] = thresholds[i];
        debug!(
            "{ware_id:?}: t=[{t0}, {t1}, {t2}, {t3}] raw ({} units of week supply), base {base_per_unit:.1}/unit, sell@t0 {}, buy@t1 {}",
            t0 / scaling,
            sell_price(ware_index, PriceLevel::Center),
            buy_price(ware_index, PriceLevel::Center),
        );
        let levels = LEVEL_KEYS.map(|(_, level)| level);
        let sells: Vec<String> = levels.iter().map(|&l| sell_price(ware_index, l).to_string()).collect();
        let buys: Vec<String> = levels.iter().map(|&l| buy_price(ware_index, l).to_string()).collect();
        csv.push_str(&format!(
            "{ware_id:?},{base_per_unit:.1},{},{},{:.1},{:.1},{:.1},{:.1},{},{},{},{}\n",
            sells.join(","),
            buys.join(","),
            citizens[i] as f32 * 7.0 / scaling as f32,
            businesses[i] as f32 * 7.0 / scaling as f32,
            production[i] as f32 / scaling as f32,
            (t2 - t1) as f32 / 10.0 / scaling as f32,
            t0 / scaling,
            t1 / scaling,
            t2 / scaling,
            t3 / scaling
        ));
    }

    let file_name = format!("{town_name}.csv");
    match std::fs::write(&file_name, csv) {
        Ok(()) => info!("wrote {file_name} to the game directory"),
        Err(e) => error!("failed to write {file_name}: {e}"),
    }
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
/// `.claude/notes/todo/captain-experience.md`. Dumps to DebugView and to `_probe1.log`
/// in the game folder - truncated on the first press of a session and appended
/// afterwards, so two presses days apart sit in one file and can be diffed.
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
