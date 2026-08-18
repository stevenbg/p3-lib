use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicPtr, AtomicU32, Ordering};
use std::{mem, panic};

use hooklet::windows::x86::{hook_function_pointer, FunctionPointerHook};
use log::error;
use num_traits::FromPrimitive;
use p3_api::{
    data::{enums::WareId, p3_ptr::P3Pointer},
    game_world::GAME_WORLD_PTR,
    operation::Operation,
    operations::{execute_operation, OPERATIONS_PTR},
    town::get_town_name,
    ui::ui_trading_office_window::UITradingOfficeWindowPtr,
};
use p3_rou::{builder, TradeRouteStop};
use windows::Win32::{
    Foundation::{LPARAM, LRESULT, WPARAM},
    System::Threading::GetCurrentThreadId,
    UI::{
        Input::KeyboardAndMouse::{GetKeyState, VIRTUAL_KEY, VK_CONTROL, VK_F1, VK_F10, VK_F11, VK_F3, VK_F4, VK_F9, VK_MENU},
        WindowsAndMessaging::{CallNextHookEx, SetWindowsHookExW, HHOOK, WH_KEYBOARD},
    },
};

use crate::prices::{buy_price, sell_price, PriceLevel};

/// F1: set every ware without an order to BUY (produced by the town) or SELL (rest),
/// at the Center price levels (buy t1, sell t0).
const SETUP_KEY: usize = VK_F1.0 as usize;

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
/// F9: log the current town (probe for the current-town global).
const CURRENT_TOWN_KEY: usize = VK_F9.0 as usize;
/// F4 appends a trade stop for the current town to the selected ship's route; Ctrl+F4
/// does the same but also buys the [NO_BUY_WARES].
const ADD_STOP_KEY: usize = VK_F4.0 as usize;
/// F3 sets a whole route template: plain 5stop, alt 6stop, ctrl suck.
const ROUTE_KEY: usize = VK_F3.0 as usize;
/// The stop buys what the town produces at this level (the Ctrl+Y price).
const STOP_BUY_LEVEL: PriceLevel = PriceLevel::LowerMid;
/// The stop sells everything else at this level (the Alt+Y price).
const STOP_SELL_LEVEL: PriceLevel = PriceLevel::LowerMid;
/// Wares plain F4 never buys, even where the town produces them: their margin does not
/// justify the cargo space early on (grain, hemp and timber are bulky loads goods), so
/// they stay in the town. Ctrl+F4 buys them too.
const NO_BUY_WARES: [WareId; 6] = [WareId::Pitch, WareId::Timber, WareId::Salt, WareId::Bricks, WareId::Grain, WareId::Hemp];
/// Wares the F3 routes never supply. For bricks and timber the t0 threshold is a fixed
/// building-material base rather than a week of demand (bricks alone are 40 loads, 400
/// barrels of hold); pig iron, pitch and hemp are industry inputs whose t0 is usually the
/// minimum floor rather than real demand. Shipping any of them in would swamp the route
/// for little gain. Whatever accumulates in the target office is still hauled home.
const NO_SUPPLY_WARES: [WareId; 5] = [WareId::Bricks, WareId::Timber, WareId::PigIron, WareId::Pitch, WareId::Hemp];

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

/// The route-panel update method (thiscall, this=panel) is a vtable-dispatched virtual
/// at 0x0048b3e0; the panel object has no plain static, so we hook its vtable slot to
/// capture `this`. The panel's +0xa0 points to a struct whose first word is the
/// selected ship index (used by the panel at 0x48c363, the Load handler at 0x48c92e).
static PANEL_METHOD_ADDRESS: u32 = 0x0048b3e0;
/// Module-relative offset of the vtable slot holding PANEL_METHOD_ADDRESS (abs 0x66f44c).
const PANEL_METHOD_VTABLE_OFFSET: u32 = 0x0026f44c;
const PANEL_SELECTION_OFFSET: u32 = 0xa0;
/// The captured panel object pointer (set by panel_capture_hook on every panel update).
static CACHED_PANEL: AtomicU32 = AtomicU32::new(0);
static PANEL_HOOK_PTR: AtomicPtr<FunctionPointerHook> = AtomicPtr::new(std::ptr::null_mut());

extern "C" {
    static panel_capture_hook: core::ffi::c_void;
}

// Save the panel object pointer (ecx = this) on every call, then run the real method.
// The jump target static holds PANEL_METHOD_ADDRESS so the vtable-installed hook runs
// the original method with the stack and registers untouched.
std::arch::global_asm!("
.global {hook}
{hook}:
mov dword ptr [{cached}], ecx
jmp [{method}]
",
    hook = sym panel_capture_hook,
    cached = sym CACHED_PANEL,
    method = sym PANEL_METHOD_ADDRESS,
);

/// Logs unconditionally via OutputDebugString: the shared DEBUGGER_LOGGER drops all
/// messages unless a real debugger is attached, which hides them from DebugView.
fn ods(message: &str) {
    win_dbg_logger::output_debug_string(&format!("auto_supply: {message}\r\n"));
}

/// Sets the PEB BeingDebugged flag so IsDebuggerPresent() returns true, unlocking the
/// gated win_dbg_logger used by every mod. No real debugger is attached, so log output
/// still reaches DebugView via OutputDebugString.
#[cfg(target_arch = "x86")]
unsafe fn fake_being_debugged() {
    let peb: *mut u8;
    std::arch::asm!("mov {}, fs:[0x30]", out(reg) peb);
    // PEB + 0x02 = BeingDebugged (u8).
    *peb.add(2) = 1;
}

#[cfg(not(target_arch = "x86"))]
unsafe fn fake_being_debugged() {}

#[no_mangle]
pub unsafe extern "C" fn start() -> u32 {
    let _ = log::set_logger(&win_dbg_logger::DEBUGGER_LOGGER);
    log::set_max_level(log::LevelFilter::Trace);

    panic::set_hook(Box::new(|p| {
        error!("{p}");
    }));

    fake_being_debugged();

    // start() runs on the game's main thread (the modloader calls it from its WinMain
    // hook), which then pumps the message loop - so a thread-scoped keyboard hook
    // installed here fires on key events at any game speed, for the process lifetime.
    match SetWindowsHookExW(WH_KEYBOARD, Some(keyboard_hook), None, GetCurrentThreadId()) {
        Ok(hook) => KEYBOARD_HOOK.store(hook.0, Ordering::SeqCst),
        Err(_) => {
            ods("installing the keyboard hook failed");
            return 1;
        }
    }

    // Track whether a trading office window is open, to gate the office-only keys. The
    // OS keyboard hook is all-or-nothing per thread, so the scoping is done in code.
    match hook_function_pointer(OFFICE_WINDOW_OPEN_POINTER_OFFSET, office_window_open_hook as usize as u32) {
        Ok(hook) => OPEN_HOOK_PTR.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
        Err(_) => {
            ods("failed to hook office window open");
            return 2;
        }
    }
    match hook_function_pointer(OFFICE_WINDOW_CLOSE_POINTER_OFFSET, office_window_close_hook as usize as u32) {
        Ok(hook) => CLOSE_HOOK_PTR.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
        Err(_) => {
            ods("failed to hook office window close");
            return 3;
        }
    }

    // Capture the route-panel object (for reading the selected ship) via its vtable slot.
    match hook_function_pointer(PANEL_METHOD_VTABLE_OFFSET, &panel_capture_hook as *const _ as u32) {
        Ok(hook) => PANEL_HOOK_PTR.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
        Err(_) => {
            ods("failed to hook the route panel method");
            return 4;
        }
    }

    ods("loaded: office F1 setup, ctrl/alt+QWERTY prices, F11 debug; global F4 add stop, F9 town+ship, F10 routes");
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
            // The office keys act only while a trading office window is open; the town
            // and route probes are global.
            let office_open = OFFICE_WINDOW_OPEN.load(Ordering::SeqCst);
            match wparam.0 {
                w if w == CURRENT_TOWN_KEY => on_current_town_hotkey(),
                w if w == ROUTE_DUMP_KEY => dump_ship_routes(),
                w if w == TOWN_DUMP_KEY => on_town_dump_hotkey(),
                // Plain F4 skips the NO_BUY_WARES, ctrl+F4 buys everything produced.
                // Alt+F4 is left to Windows.
                w if w == ADD_STOP_KEY && !alt => on_add_stop_hotkey(!ctrl),
                w if w == ROUTE_KEY => on_route_hotkey(if ctrl {
                    RouteKind::Suck
                } else if alt {
                    RouteKind::SixStop
                } else {
                    RouteKind::FiveStop
                }),
                _ if !office_open => {}
                w if w == SETUP_KEY => on_setup_hotkey(),
                // Exactly one of ctrl (buy) / alt (sell) picks the direction.
                w if ctrl != alt => {
                    if let Some(&(_, level)) = LEVEL_KEYS.iter().find(|(key, _)| *key == w) {
                        if ctrl {
                            apply_prices(None, Some(level));
                        } else {
                            apply_prices(Some(level), None);
                        }
                    }
                }
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

/// F9: log the current town, the player's home town and the selected ship (diagnostics
/// for the route feature).
unsafe fn on_current_town_hotkey() {
    if let Some(town_index) = current_town_index() {
        let town = get_town_name(town_index).unwrap_or_else(|| "<unknown>".into());
        ods(&format!("current town: {town} ({town_index:#04x})"));
    }

    let merchant = GAME_WORLD_PTR.get_merchant(OPERATIONS_PTR.get_player_merchant_index() as u16);
    let home_index = merchant.get_hometown_index();
    let home = get_town_name(home_index).unwrap_or_else(|| "<unknown>".into());
    ods(&format!("home town: {home} ({home_index:#04x})"));

    probe_selected_ship();
}

/// The ship currently shown in the route panel (map/panel selection), via the captured
/// panel object: panel+0xa0 points to a selection struct whose first word is the index.
unsafe fn selected_ship_index() -> Option<u16> {
    let panel = CACHED_PANEL.load(Ordering::Relaxed);
    if panel == 0 {
        return None;
    }
    let selection = *((panel + PANEL_SELECTION_OFFSET) as *const u32);
    if !(0x0010_0000..0x7f00_0000).contains(&selection) {
        return None;
    }
    Some(*(selection as *const u16))
}

/// Read the selected ship via the captured panel object and log it.
unsafe fn probe_selected_ship() {
    let ships = p3_api::ships::ShipsPtr::new();
    match selected_ship_index() {
        Some(index) => {
            let name = ships.get_ship(index).map(|s| s.get_name()).unwrap_or_default();
            ods(&format!("selected ship: {index} {name:?}"));
        }
        None => ods("selected ship: none captured (select a ship)"),
    }
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
unsafe fn read_ship_route(ship_index: u16) -> Vec<TradeRouteStop> {
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
    let mut stops = Vec::new();
    let mut idx = head;
    for _ in 0..64 {
        let record = pool + idx as u32 * ROUTE_STOP_SIZE;
        stops.push(read_pool_stop(record));
        let next = *(record as *const u16);
        if next == idx || next >= pool_count {
            break;
        }
        idx = next;
        if idx == head {
            break;
        }
    }
    if let Some(pos) = stops.iter().position(|s| s.action & FIRST_STOP_MARKER != 0) {
        stops.rotate_left(pos);
    }
    stops
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
    ods("add stop: name string built");

    *OPERATIONS_ROUTE_SHIP = ship_index as u32;
    let loader: extern "thiscall" fn(u32, *const u32) -> u32 = mem::transmute(ROUTE_LOADER_ADDRESS);
    let buffer = loader(ROUTE_LOADER_THIS, &string_object);
    ods(&format!("add stop: loader returned buffer {buffer:#010x}"));

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
        ods(&format!("{what}: no ship selected"));
        return None;
    };
    if !is_player_ship(ship_index) {
        ods(&format!("{what}: the selected ship is not yours"));
        return None;
    }
    let Some(town_index) = current_town_index() else {
        ods(&format!("{what}: not in a town"));
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
        ods(&format!("route: failed to write {ROUTE_FILE_PATH}: {e}"));
        return false;
    }
    if !apply_route_file(ship_index) {
        ods("route: loader failed (is fix_uncompressed_trade_route_loading.dll installed?)");
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
        ods(&format!(
            "add stop in {town} on {name:?}: buying [{}]{skipped}, selling {sold} others (route now {stop_count} stops)",
            bought.join(", ")
        ));
    }
}

/// The route templates F3 can set, from `p3_rou::builder`.
#[derive(Clone, Copy, Debug)]
enum RouteKind {
    FiveStop,
    SixStop,
    Suck,
}

/// F3: replace the selected ship's route with a supply route template: load one week of
/// the current town's demand at the home office, sell it there, reset that office's stock
/// and haul the surplus home. Ctrl+F3 instead parks in the current town buying everything
/// up. Quantities come from the town's t0 thresholds (a week of citizen and business
/// demand, in raw units) and prices from the R levels; wares the town produces itself are
/// not supplied to it.
unsafe fn on_route_hotkey(kind: RouteKind) {
    let Some((ship_index, name, sell_town)) = route_context("route") else {
        return;
    };
    let merchant = GAME_WORLD_PTR.get_merchant(OPERATIONS_PTR.get_player_merchant_index() as u16);
    let load_town = merchant.get_hometown_index();
    let sell_town_name = get_town_name(sell_town).unwrap_or_else(|| "<unknown>".into());
    let load_town_name = get_town_name(load_town).unwrap_or_else(|| "<unknown>".into());

    let town = GAME_WORLD_PTR.get_town(sell_town);
    let thresholds = town.get_price_thresholds();
    let production = town.get_production_values();

    let mut load_amount = [0i32; 24];
    let mut sell_prices = [0i32; 24];
    let mut buy_prices = [0i32; 24];
    let mut supplied = 0;
    for ware_index in TRADE_WARES {
        let i = ware_index as usize;
        let ware_id = WareId::from_u16(ware_index).unwrap();
        buy_prices[i] = buy_price(ware_index, PriceLevel::Center);
        // A zero load amount is how the route templates express "do not supply this":
        // the ware is neither loaded nor sold nor put back, but whatever the target
        // office holds is still collected.
        if production[i] > 0 || NO_SUPPLY_WARES.contains(&ware_id) {
            continue;
        }
        // t0 is one week of the town's demand, already in raw units.
        load_amount[i] = thresholds[i][0];
        sell_prices[i] = sell_price(ware_index, PriceLevel::Center);
        supplied += 1;
    }

    // These templates replace the whole route, so keep the old one loadable from the
    // route window's File button ("_backup") in case this was a mistake.
    let previous = read_ship_route(ship_index);
    if !previous.is_empty() {
        let count = previous.len();
        let backup = p3_rou::TradeRouteFile { stops: previous }.serialize();
        match std::fs::write(ROUTE_BACKUP_PATH, &backup) {
            Ok(()) => ods(&format!("route: saved the previous {count} stops to {ROUTE_BACKUP_PATH}")),
            Err(e) => ods(&format!("route: failed to back up the previous route: {e}")),
        }
    }

    let stops = match kind {
        RouteKind::FiveStop => builder::five_stop_route(load_town, sell_town, load_amount, sell_prices),
        RouteKind::SixStop => builder::six_stop_route(load_town, sell_town, load_amount, sell_prices),
        RouteKind::Suck => builder::suck_route(sell_town, buy_prices),
    };
    let stop_count = stops.len();

    // Every template transfers wares to or from an office in the current town, and the
    // game wipes those instructions at load time where there is no office. The load town
    // is the home office, so only the current town can be missing one.
    if !has_player_office(sell_town) {
        ods(&format!(
            "route: no office in {sell_town_name} - the game will wipe this route's office transfers there"
        ));
    }

    if write_and_apply_route(ship_index, stops) {
        match kind {
            RouteKind::Suck => ods(&format!(
                "route {kind:?} on {name:?}: parked in {sell_town_name} buying everything at the R prices ({stop_count} stops)"
            )),
            _ => ods(&format!(
                "route {kind:?} on {name:?}: load in {load_town_name}, supply {supplied} wares to {sell_town_name} ({stop_count} stops)"
            )),
        }
    }
}

/// Returns the administrator view's office and its index, or logs why not.
unsafe fn resolve_office() -> Option<(p3_api::data::office::OfficePtr, u16, String)> {
    let window = UITradingOfficeWindowPtr::new();
    if window.get_address() == 0 {
        ods("hotkey: no trading office window (open one on the Trading Office view)");
        return None;
    }
    let town_index = window.get_town_index();
    let town = get_town_name(town_index as u8).unwrap_or_else(|| "<unknown>".into());

    let page = window.get_selected_page();
    if page != ADMINISTRATOR_PAGE {
        ods(&format!("hotkey: office window in {town}, page {page} - switch to the Trading Office view"));
        return None;
    }

    let merchant_index = OPERATIONS_PTR.get_player_merchant_index();
    let Some(office) = GAME_WORLD_PTR.get_office_in_of(town_index as _, merchant_index as _) else {
        ods(&format!("hotkey: no player office found in {town}"));
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
    ods(&format!(
        "setup in {town}: buying produced wares [{}], selling {sold} others, {untouched} existing orders untouched",
        bought.join(", ")
    ));
    refresh_administrator_view();
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
    ods(&format!("prices (sell {sell:?}, buy {buy:?}): updated {updated} wares in {town}"));
    refresh_administrator_view();
}

/// Re-selects the administrator page, which rebuilds the direction arrows and prices.
/// The displayed amounts are cached in widget objects that only a full window reopen
/// rebuilds - known cosmetic limitation, the office data itself is correct.
unsafe fn refresh_administrator_view() {
    UITradingOfficeWindowPtr::new().select_new_page(ADMINISTRATOR_PAGE);
}

/// F11: dump thresholds, base prices and all price levels for the open town to the log
/// and to <TownName>.csv in the game directory.
unsafe fn on_town_dump_hotkey() {
    let Some(town_index) = current_town_index() else {
        ods("debug dump: not in a town");
        return;
    };
    let town_name = get_town_name(town_index).unwrap_or_else(|| "<unknown>".into());
    let thresholds = GAME_WORLD_PTR.get_town(town_index).get_price_thresholds();
    let production = GAME_WORLD_PTR.get_town(town_index).get_production_values();

    ods(&format!(
        "debug dump for {town_name} (live trade difficulty {}):",
        crate::prices::difficulty_d()
    ));
    // The price columns follow the hotkeys Q W E R T Y (ascending price), sells (alt)
    // then buys (ctrl).
    let mut csv = String::from(
        "ware,base/unit,\
         sell_Q_t1,sell_W_mid_t0_t1,sell_E_30_t0_t1,sell_R_t0,sell_T_70_0_t0,sell_Y_mid_0_t0,\
         buy_Q_t2,buy_W_mid_t1_t2,buy_E_30_t1_t2,buy_R_t1,buy_T_70_t0_t1,buy_Y_mid_t0_t1,\
         prod/day,(t2-t1)/10,t0 units,t1 units,t2 units,t3 units\n",
    );
    for ware_index in TRADE_WARES {
        let i = ware_index as usize;
        let ware_id = WareId::from_u16(ware_index).unwrap();
        let scaling = ware_id.get_scaling();
        let base_per_unit = crate::prices::base_price_per_unit(ware_index);
        let [t0, t1, t2, t3] = thresholds[i];
        ods(&format!(
            "{ware_id:?}: t=[{t0}, {t1}, {t2}, {t3}] raw ({} units of week supply), base {base_per_unit:.1}/unit, sell@t0 {}, buy@t1 {}",
            t0 / scaling,
            sell_price(ware_index, PriceLevel::Center),
            buy_price(ware_index, PriceLevel::Center),
        ));
        let levels = LEVEL_KEYS.map(|(_, level)| level);
        let sells: Vec<String> = levels.iter().map(|&l| sell_price(ware_index, l).to_string()).collect();
        let buys: Vec<String> = levels.iter().map(|&l| buy_price(ware_index, l).to_string()).collect();
        csv.push_str(&format!(
            "{ware_id:?},{base_per_unit:.1},{},{},{:.1},{:.1},{},{},{},{}\n",
            sells.join(","),
            buys.join(","),
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
        Ok(()) => ods(&format!("wrote {file_name} to the game directory")),
        Err(e) => ods(&format!("failed to write {file_name}: {e}")),
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

/// F10: walk every ship's route chain and dump the stops.
unsafe fn dump_ship_routes() {
    let pool = *ROUTE_STOP_POOL;
    let pool_count = *ROUTE_STOP_POOL_COUNT;
    ods(&format!("route stop pool at {pool:#010x}, {pool_count} entries"));
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
        ods(&format!("ship {ship_id} at {:#010x} {:?}: route head {head}", ship.address, ship.get_name()));

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
            ods(&format!(
                "  stop {n}: pool[{index}] town {town}, action {action:#04x}, next {next}, ops [{}]",
                ops.join(", ")
            ));
            index = next;
            if index == head || index >= pool_count {
                break;
            }
        }
    }
}
