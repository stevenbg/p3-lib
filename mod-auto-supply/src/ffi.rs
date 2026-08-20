use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicPtr, AtomicU32, Ordering};
use std::{mem, panic};

use hooklet::windows::x86::{hook_call_rel32, hook_function_pointer, CallRel32Hook, FunctionPointerHook};
use log::{debug, error, info};
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
/// F9: throwaway debug logic for the current RE task (see on_current_town_hotkey).
const CURRENT_TOWN_KEY: usize = VK_F9.0 as usize;
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

    // Capture the route-panel object (for reading the selected ship) via its vtable slot.
    match hook_function_pointer(PANEL_METHOD_VTABLE_OFFSET, &panel_capture_hook as *const _ as u32) {
        Ok(hook) => PANEL_HOOK_PTR.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
        Err(_) => {
            error!("failed to hook the route panel method");
            return 4;
        }
    }

    info!("loaded: office F1 setup, ctrl/alt+QWERTY prices, F11 debug; global F4 add stop, F9 town+ship, F10 routes");
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
                w if w == CURRENT_TOWN_KEY => on_current_town_hotkey(),
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

unsafe fn on_current_town_hotkey() {
    find_tavern_captains();
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
    let order = builder::cargo_order(&price, &amount);
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
        notify(&format!("Office keys: switch to the Trading Office view ({town} is on page {page})"));
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
