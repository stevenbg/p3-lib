use std::sync::atomic::{AtomicPtr, AtomicU32, Ordering};
use std::{mem, panic};

use hooklet::windows::x86::{hook_call_rel32, hook_function_pointer, CallRel32Hook, FunctionPointerHook};
use log::{debug, error, info, warn};
use num_traits::FromPrimitive;
use p3_api::{
    data::{enums::WareId, p3_ptr::P3Pointer},
    game_world::GAME_WORLD_PTR,
    hotkeys::{HotkeyHandler, HotkeysApi, MOD_ALT, MOD_CTRL, MOD_SHIFT},
    operation::Operation,
    operations::{execute_operation, OPERATIONS_PTR},
    scheduled_tasks::{scheduled_task::SCHEDULED_TASK_OPCODE_UNFREEZE_PORT, SCHEDULED_TASKS_PTR},
    town::get_town_name,
    ui::{ui_ship_panel::UIShipPanelPtr, ui_trading_office_window::UITradingOfficeWindowPtr},
};
use p3_rou::{builder, builder::MAX_DISPLAYABLE_STOPS, TradeRouteStop};
use windows::Win32::UI::Input::KeyboardAndMouse::{VK_DELETE, VK_F1, VK_F11, VK_F2, VK_F3, VK_F4};

use crate::prices::{buy_price, sell_price, PriceLevel};

/// F1: set every ware without an order to BUY (produced by the town) or SELL (rest),
/// at the Center price levels (buy t1, sell t0). Ctrl+F1: provision and lock the
/// celebration goods; Alt+F1: provision and lock the building materials.
const SETUP_KEY: u32 = VK_F1.0 as u32;
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
/// Buying gets more aggressive towards Y (pays more, so it drains the town deeper),
/// selling gets more restrained (holds out for more, so it only sells into scarcity).
/// Q is the natural pair - buy at par, sell at the supply price - and also the widest
/// margin; see [PriceLevel] for the whole ladder.
const LEVEL_KEYS: [(u32, PriceLevel); 6] = [
    (0x51, PriceLevel::Q), // buy 1.00 (t1, par) / sell 1.40 (t0, the supply price)
    (0x57, PriceLevel::W), // buy 1.05 / sell 1.45
    (0x45, PriceLevel::E), // buy 1.10 / sell 1.50
    (0x52, PriceLevel::R), // buy 1.15 / sell 1.55
    (0x54, PriceLevel::T), // buy 1.20 / sell 1.60
    (0x59, PriceLevel::Y), // buy 1.25 (mid t0..t1) / sell 1.65
];
/// F11: dump the current town's thresholds, base prices and price levels to the log and
/// to a CSV.
const TOWN_DUMP_KEY: u32 = VK_F11.0 as u32;
/// The trade template: one self-contained trade stop per target.
const ROUTE_TRADE_KEY: u32 = VK_F4.0 as u32;
/// The route templates get one key each, F1/F2/F3, with SHIFT appending to the existing
/// route instead of replacing it.
///
/// F1 is deliberately shared with [SETUP_KEY]: the office and goods-dialog groups
/// register F1 too, and because the registry dispatches newest-first their registration
/// **shadows** this one while their window is open - which is what the shared registry was
/// built for. Those scoped handlers swallow the keystroke (return nonzero) so exactly one
/// thing happens per press; when the window closes and its registration goes, the global
/// route key resurfaces on its own. Nothing here has to know whether a window is open.
const ROUTE_5STOP_KEY: u32 = VK_F1.0 as u32;
/// The 6stop office-swap template.
const ROUTE_6STOP_KEY: u32 = VK_F2.0 as u32;
/// The collection ("suck") template. With CTRL, the fetch template: the same collection
/// shape, but the wares come from the ship's own first stop and the targets from who
/// produces them.
const ROUTE_SUCK_KEY: u32 = VK_F3.0 as u32;
/// DEL clears the selected ship's route (guarded on the goods dialog being closed).
const CLEAR_ROUTE_KEY: u32 = VK_DELETE.0 as u32;
// The price levels the generated orders use. Every automatic price in the mod comes from
// one of these five, so retuning is a one-line change per feature:
//
// | feature | buy | sell |
// |-|-|-|
// | F1 office setup | Q (par, 1.00) | Q (supply price, 1.40) |
// | F1 goods-dialog fill | [STOP_BUY_LEVEL] R | [STOP_SELL_LEVEL] R |
// | F4 trade template | [STOP_BUY_LEVEL] R | [STOP_SELL_LEVEL] R |
// | F1/F2 supply templates | [COLLECT_BUY_LEVEL] E | [SUPPLY_SELL_LEVEL] Q |
// | F3 collection template | [COLLECT_BUY_LEVEL] E | - |
//
// The CTRL+F3 fetch template is deliberately absent: it copies the buy prices off the
// ship's own first stop, so its prices are the player's, not a level of ours. Which is
// also how it gets a per-ware price instead of one letter for the whole list.
//
// The office setup's Q is written at its own call site because it prices existing orders
// rather than building a stop.

/// What a trade stop pays for the wares its town produces - the F4 template and the
/// goods-dialog fill, which build the same shape of stop.
const STOP_BUY_LEVEL: PriceLevel = PriceLevel::R;
/// What a trade stop asks for everything its town does not produce.
const STOP_SELL_LEVEL: PriceLevel = PriceLevel::R;
/// What the collection route and the office-less targets of a supply route pay. Shared
/// deliberately: both are "haul home what this town has that we do not produce".
const COLLECT_BUY_LEVEL: PriceLevel = PriceLevel::E;
/// What a supply route asks for the goods it delivers to a target.
const SUPPLY_SELL_LEVEL: PriceLevel = PriceLevel::Q;
/// Wares the trade template (F4) and the goods-dialog fill leave in the town even where
/// it produces them: their margin does not justify the cargo space early on (grain, hemp
/// and timber are bulky loads goods). ALT buys them too. Also the wares the collection
/// route never buys - there unconditionally.
const NO_BUY_WARES: [WareId; 6] = [WareId::Pitch, WareId::Timber, WareId::Salt, WareId::Bricks, WareId::Grain, WareId::Hemp];
/// Low-value industry inputs, worth skipping when hold space is tight: what a target
/// consumes of them is mostly business demand, and they crowd out goods with a better
/// margin. **Excluded only when ALT is held** with a route key - a plain route supplies
/// everything the target consumes. Whatever accumulates in the target office is hauled
/// home either way. (Timber is not on this list - as the worst of the loads goods it is
/// simply ordered last, see p3-rou's cargo_order.)
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
/// Amount set for buy orders whose current amount is 0, in the in-game display units the
/// office shows: loads for load wares, barrels for barrel wares. The two are the same
/// physical quantity - a load is ten barrels, so both scale to 40,000 raw - so a setup
/// asks for an equal amount of everything regardless of how it is measured.
const BUY_AMOUNT_LOADS: i32 = 20;
const BUY_AMOUNT_BARRELS: i32 = 200;
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

static OPEN_HOOK_PTR: AtomicPtr<FunctionPointerHook> = AtomicPtr::new(std::ptr::null_mut());
static CLOSE_HOOK_PTR: AtomicPtr<FunctionPointerHook> = AtomicPtr::new(std::ptr::null_mut());
static DIALOG_CLOSE_HOOK_PTR: AtomicPtr<FunctionPointerHook> = AtomicPtr::new(std::ptr::null_mut());
static POPULATE_HOOKS: [AtomicPtr<CallRel32Hook>; 3] = [
    AtomicPtr::new(std::ptr::null_mut()),
    AtomicPtr::new(std::ptr::null_mut()),
    AtomicPtr::new(std::ptr::null_mut()),
];

/// The shared hotkey registry (hotkeys.dll), bound at start(); null = unavailable,
/// keys inert. All key dispatch goes through it - this mod installs no keyboard
/// hook of its own any more.
static HOTKEYS: AtomicPtr<HotkeysApi> = AtomicPtr::new(std::ptr::null_mut());

unsafe fn hotkeys() -> Option<&'static HotkeysApi> {
    HOTKEYS.load(Ordering::SeqCst).as_ref()
}

/// The three lifetime groups. Handles are 0 when unregistered; registration always
/// unregisters a stale handle first, so a double-fired open cannot orphan one.
const OWNER_GLOBAL: &std::ffi::CStr = c"auto-supply";
const OWNER_OFFICE: &std::ffi::CStr = c"auto-supply office";
const OWNER_DIALOG: &std::ffi::CStr = c"auto-supply goods dialog";

/// Session-global keys, registered once at start(): the route keys and the town
/// dump. (The F9/F10 debug probes live in mod-crash-reporter now.) The handlers
/// never swallow, exactly like the old all-seeing hook, which always fell through
/// to CallNextHookEx.
const GLOBAL_KEYS: [(u32, u32); 20] = [
    (TOWN_DUMP_KEY, 0),
    (ROUTE_TRADE_KEY, 0),
    (ROUTE_TRADE_KEY, MOD_SHIFT),
    (ROUTE_TRADE_KEY, MOD_ALT),
    (ROUTE_TRADE_KEY, MOD_ALT | MOD_SHIFT),
    (ROUTE_5STOP_KEY, 0),
    (ROUTE_5STOP_KEY, MOD_SHIFT),
    (ROUTE_5STOP_KEY, MOD_ALT),
    (ROUTE_5STOP_KEY, MOD_ALT | MOD_SHIFT),
    (ROUTE_6STOP_KEY, 0),
    (ROUTE_6STOP_KEY, MOD_SHIFT),
    (ROUTE_6STOP_KEY, MOD_ALT),
    (ROUTE_6STOP_KEY, MOD_ALT | MOD_SHIFT),
    (ROUTE_SUCK_KEY, 0),
    (ROUTE_SUCK_KEY, MOD_SHIFT),
    (ROUTE_SUCK_KEY, MOD_ALT),
    (ROUTE_SUCK_KEY, MOD_ALT | MOD_SHIFT),
    // The fetch template. ALT relaxes a filter here as everywhere else, but the TOWN
    // filter rather than a ware one: it calls at every town instead of only the producers.
    // No SHIFT variant - the targets are derived, so SHIFT's single open town has nothing
    // to say.
    (ROUTE_SUCK_KEY, MOD_CTRL),
    (ROUTE_SUCK_KEY, MOD_CTRL | MOD_ALT),
    (CLEAR_ROUTE_KEY, 0),
];
static GLOBAL_HANDLES: [AtomicU32; 20] = [const { AtomicU32::new(0) }; 20];

/// Office keys, registered while a trading office window is open (its vtable
/// open/close hooks below): F1 and the price-level keys.
const OFFICE_KEYS: [(u32, u32); 16] = [
    (SETUP_KEY, 0),
    (CLEAR_ROUTE_KEY, 0),
    (SETUP_KEY, MOD_CTRL),
    (SETUP_KEY, MOD_ALT),
    (LEVEL_KEYS[0].0, MOD_CTRL),
    (LEVEL_KEYS[0].0, MOD_ALT),
    (LEVEL_KEYS[1].0, MOD_CTRL),
    (LEVEL_KEYS[1].0, MOD_ALT),
    (LEVEL_KEYS[2].0, MOD_CTRL),
    (LEVEL_KEYS[2].0, MOD_ALT),
    (LEVEL_KEYS[3].0, MOD_CTRL),
    (LEVEL_KEYS[3].0, MOD_ALT),
    (LEVEL_KEYS[4].0, MOD_CTRL),
    (LEVEL_KEYS[4].0, MOD_ALT),
    (LEVEL_KEYS[5].0, MOD_CTRL),
    (LEVEL_KEYS[5].0, MOD_ALT),
];
static OFFICE_HANDLES: [AtomicU32; 16] = [const { AtomicU32::new(0) }; 16];

/// Goods-dialog keys, registered after every populate (the dialog's open - the
/// Goods button and the dialog's own arrows) and unregistered on its close: F1 fill
/// and the price-level keys, repricing the displayed stop.
const DIALOG_KEYS: [(u32, u32); 14] = [
    (SETUP_KEY, 0),
    (SETUP_KEY, MOD_CTRL),
    (LEVEL_KEYS[0].0, MOD_CTRL),
    (LEVEL_KEYS[0].0, MOD_ALT),
    (LEVEL_KEYS[1].0, MOD_CTRL),
    (LEVEL_KEYS[1].0, MOD_ALT),
    (LEVEL_KEYS[2].0, MOD_CTRL),
    (LEVEL_KEYS[2].0, MOD_ALT),
    (LEVEL_KEYS[3].0, MOD_CTRL),
    (LEVEL_KEYS[3].0, MOD_ALT),
    (LEVEL_KEYS[4].0, MOD_CTRL),
    (LEVEL_KEYS[4].0, MOD_ALT),
    (LEVEL_KEYS[5].0, MOD_CTRL),
    (LEVEL_KEYS[5].0, MOD_ALT),
];
static DIALOG_HANDLES: [AtomicU32; 14] = [const { AtomicU32::new(0) }; 14];

unsafe fn register_group(owner: &'static std::ffi::CStr, keys: &[(u32, u32)], handles: &[AtomicU32], handler: HotkeyHandler) {
    let Some(api) = hotkeys() else { return };
    for (i, &(vk, mods)) in keys.iter().enumerate() {
        let stale = handles[i].swap(0, Ordering::SeqCst);
        if stale != 0 {
            api.unregister(stale);
        }
        handles[i].store(api.register(owner, vk, mods, handler), Ordering::SeqCst);
    }
}

unsafe fn unregister_group(handles: &[AtomicU32]) {
    let Some(api) = hotkeys() else { return };
    for handle in handles {
        let handle = handle.swap(0, Ordering::SeqCst);
        if handle != 0 {
            api.unregister(handle);
        }
    }
}

/// Post an in-game popup on the event ticker (the top-left "Game speed:" boxes),
/// mirrored to the debug log.
pub(crate) unsafe fn notify(text: &str) {
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

    // All keys go through the shared registry (hotkeys.dll); a missing or stale
    // registry degrades to inert keys, never to a load failure.
    match HotkeysApi::bind() {
        Ok(api) => {
            HOTKEYS.store(Box::into_raw(Box::new(api)), Ordering::SeqCst);
            register_group(OWNER_GLOBAL, &GLOBAL_KEYS, &GLOBAL_HANDLES, global_hotkeys);
        }
        Err(reason) => warn!("hotkeys registry unavailable ({reason}) - all keys inert"),
    }

    if let Err(reason) = crate::crew_rescue::install() {
        warn!("crew rescue unavailable ({reason}) - the game's own message stands");
    }

    // The office keys are registered while a trading office window is open, off its
    // vtable open/close (close fires on every path - play-verified for the hotkey
    // registry design).
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

    // The goods-dialog keys are registered after populate (the dialog's open: the
    // panel's Goods button and the dialog's own arrows - populate's three call
    // sites) and unregistered on the dialog's close. Registration happens AFTER the
    // original returns because populate itself closes an already-open dialog first,
    // which unregisters; the post-call order makes that harmless.
    for (i, offset) in DIALOG_POPULATE_CALL_OFFSETS.iter().enumerate() {
        match hook_call_rel32(*offset, dialog_populate_hook as usize as u32) {
            Ok(hook) => POPULATE_HOOKS[i].store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
            Err(_) => {
                error!("failed to hook goods dialog populate call at module+{offset:#x}");
                return 4;
            }
        }
    }
    match hook_function_pointer(DIALOG_CLOSE_POINTER_OFFSET, dialog_close_hook as usize as u32) {
        Ok(hook) => DIALOG_CLOSE_HOOK_PTR.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
        Err(_) => {
            error!("failed to hook the goods dialog close slot");
            return 5;
        }
    }

    // A thawing port restarts automatic trade on the route ships that serve it. Not a
    // hotkey feature: it has to fire on the game's own event, whenever that happens.
    match hook_call_rel32(UNFREEZE_PORT_CALL_OFFSET, unfreeze_port_hook as usize as u32) {
        Ok(hook) => UNFREEZE_HOOK_PTR.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
        Err(_) => {
            // Not fatal: every hotkey feature still works without it.
            error!("failed to hook the unfreeze-port task - thawed ports will not restart ships");
        }
    }

    info!("loaded: global route templates F1 5stop / F2 6stop / F3 collect / F4 trade / CTRL+F3 fetch (shift appends, alt relaxes each template's filter), DEL, F11; office and goods-dialog F1 shadow the global F1 while open; thaw restarts route ships");
    0
}

#[no_mangle]
unsafe extern "thiscall" fn office_window_open_hook(window_address: u32) {
    let orig: extern "thiscall" fn(u32) = mem::transmute((*OPEN_HOOK_PTR.load(Ordering::SeqCst)).old_absolute);
    orig(window_address);
    register_group(OWNER_OFFICE, &OFFICE_KEYS, &OFFICE_HANDLES, office_hotkeys);
}

#[no_mangle]
unsafe extern "thiscall" fn office_window_close_hook(window_address: u32) {
    let orig: extern "thiscall" fn(u32) = mem::transmute((*CLOSE_HOOK_PTR.load(Ordering::SeqCst)).old_absolute);
    orig(window_address);
    unregister_group(&OFFICE_HANDLES);
}

/// Populate = the goods dialog opening (or moving to another stop). thiscall with
/// three stack arguments, `ret 0xC`; the original is [DIALOG_POPULATE] itself.
#[no_mangle]
unsafe extern "thiscall" fn dialog_populate_hook(dialog: u32, stop_index: u32, ship_index: u32, flag: u32) {
    let original: extern "thiscall" fn(u32, u32, u32, u32) = mem::transmute(DIALOG_POPULATE);
    original(dialog, stop_index, ship_index, flag);
    register_group(OWNER_DIALOG, &DIALOG_KEYS, &DIALOG_HANDLES, dialog_hotkeys);
}

/// The dialog's close (vtable `+0x118` = `0x004066F0`) fires on every leave path,
/// including defensively at the session teardown - play-verified.
#[no_mangle]
unsafe extern "thiscall" fn dialog_close_hook(dialog: u32) {
    let orig: extern "thiscall" fn(u32) = mem::transmute((*DIALOG_CLOSE_HOOK_PTR.load(Ordering::SeqCst)).old_absolute);
    orig(dialog);
    unregister_group(&DIALOG_HANDLES);
}

/// The price level a Q..Y key stands for.
fn level_of(vk: u32) -> Option<PriceLevel> {
    LEVEL_KEYS.iter().find(|&&(key, _)| key == vk).map(|&(_, level)| level)
}

/// Session-global keys: route keys and the town dump. Every handler in this mod
/// returns 0 (decline): the old keyboard hook never swallowed a key, so the game
/// and other mods keep seeing every keystroke exactly as before.
#[no_mangle]
unsafe extern "C" fn global_hotkeys(vk: u32, mods: u32) -> u32 {
    match (vk, mods) {
        (TOWN_DUMP_KEY, 0) => on_town_dump_hotkey(),
        (ROUTE_TRADE_KEY, m) => {
            on_route_hotkey(RouteKind::Trade, m & MOD_SHIFT != 0, m & MOD_ALT != 0);
            if m & MOD_ALT != 0 {
                // The only global key that swallows, and not for shadowing: ALT+F4 is
                // the OS close-window chord, and letting it travel on to
                // DefWindowProc could end the session. The game appears to ignore it,
                // but "appears to" is not worth a lost game, and we have handled the
                // key anyway.
                return 1;
            }
        }
        // One key per template. SHIFT appends the generated route to the existing one
        // instead of replacing it; ALT narrows the supply basket to skip the
        // [NO_SUPPLY_WARES]. F1 only reaches here when no office window or goods dialog
        // is open - theirs shadows it and swallows the key.
        (ROUTE_5STOP_KEY, m) => {
            on_route_hotkey(RouteKind::FiveStop, m & MOD_SHIFT != 0, m & MOD_ALT != 0)
        }
        (ROUTE_6STOP_KEY, m) => {
            on_route_hotkey(RouteKind::SixStop, m & MOD_SHIFT != 0, m & MOD_ALT != 0)
        }
        // CTRL turns F3 into the fetch template, which reads its wares off the ship - so
        // it never appends, and ALT widens its target list to every town.
        (ROUTE_SUCK_KEY, m) if m & MOD_CTRL != 0 => on_route_hotkey(RouteKind::Fetch, false, m & MOD_ALT != 0),
        (ROUTE_SUCK_KEY, m) => on_route_hotkey(RouteKind::Suck, m & MOD_SHIFT != 0, m & MOD_ALT != 0),
        // Internally guarded on the goods dialog being closed, as before.
        (CLEAR_ROUTE_KEY, 0) => on_clear_route_hotkey(),
        _ => {}
    }
    0
}

/// Office keys, live only while a trading office window is open. The handlers still
/// resolve the office themselves, so a stray press during teardown is a no-op.
///
/// **These swallow the keystroke.** F1 and DEL are also global keys, and the registry
/// dispatches newest-first, so declining here would let the global handler fire too and
/// build a route while setting up the office - or, for DEL, wipe the selected ship's route
/// while resetting the office. Returning nonzero stops the walk, which is what makes the
/// shadowing work; ownership is the rule, not success - both keys belong to the office
/// window while it is open even when the action refuses (wrong view), because refusing
/// with a popup is a better answer than silently doing the other thing.
#[no_mangle]
unsafe extern "C" fn office_hotkeys(vk: u32, mods: u32) -> u32 {
    match (vk, mods) {
        (CLEAR_ROUTE_KEY, 0) => on_reset_office_hotkey(),
        (SETUP_KEY, MOD_CTRL) => on_lock_staples_hotkey(),
        (SETUP_KEY, MOD_ALT) => on_lock_building_materials_hotkey(),
        (SETUP_KEY, 0) => on_setup_hotkey(),
        // Ctrl = buy prices, alt = sell prices, in the administrator view.
        _ => match level_of(vk) {
            Some(level) => {
                let (sell, buy) = if mods & MOD_CTRL != 0 { (None, Some(level)) } else { (Some(level), None) };
                apply_prices(sell, buy);
            }
            // Not one of ours after all: decline, so nothing downstream is starved.
            None => return 0,
        },
    }
    1
}

/// Goods-dialog keys, live from populate to close; they act on the displayed stop,
/// which the handler re-reads at press time.
///
/// **These swallow the keystroke**, for the same reason as [office_hotkeys] - F1 is a
/// global route key and the dialog's registration shadows it. The swallow happens even
/// when the displayed stop cannot be resolved: while the dialog is open these keys are
/// its own, and falling through to "build a whole route instead" would be a nasty
/// surprise.
#[no_mangle]
unsafe extern "C" fn dialog_hotkeys(vk: u32, mods: u32) -> u32 {
    // Not one of ours: decline before anything else, so nothing downstream is starved.
    if vk != SETUP_KEY && level_of(vk).is_none() {
        return 0;
    }
    let Some((dialog, stop_index)) = goods_dialog_stop() else {
        return 1;
    };
    match (vk, mods) {
        // F1: fill the displayed stop's empty slots with buy/sell orders (plain
        // skips the NO_BUY_WARES, ctrl buys everything).
        (SETUP_KEY, m) => on_dialog_setup_hotkey(dialog, stop_index, m & MOD_CTRL == 0),
        _ => {
            if let Some(level) = level_of(vk) {
                let (sell, buy) = if mods & MOD_CTRL != 0 { (None, Some(level)) } else { (Some(level), None) };
                reprice_dialog_stop(dialog, stop_index, sell, buy);
            }
        }
    }
    1
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
/// Populate's three call sites (module-relative, as hook_call_rel32 takes them): the
/// dialog's own arrows (0x004075E3, 0x0040763A) and the panel's Goods button
/// (0x0048C432). Wrapping them is the dialog's "open" event.
const DIALOG_POPULATE_CALL_OFFSETS: [u32; 3] = [0x75E3, 0x763A, 0x8C432];
/// The dialog's close, vtable slot +0x118 of 0x0066A7F0 (module-relative pointer
/// location, as hook_function_pointer takes it).
const DIALOG_CLOSE_POINTER_OFFSET: u32 = 0x26A7F0 + 0x118;

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
    // This direct call bypasses the three hooked call sites, but populate still
    // closes the open dialog internally - which unregisters the dialog keys through
    // the close hook. Re-register, exactly like the hooked sites do post-call;
    // without this, the first dialog hotkey (whose handler refreshes the dialog)
    // silently disarms all the others.
    register_group(OWNER_DIALOG, &DIALOG_KEYS, &DIALOG_HANDLES, dialog_hotkeys);
}

/// The ship currently shown in the ship panel (the map selection).
unsafe fn selected_ship_index() -> Option<u16> {
    UIShipPanelPtr::new().get_selected_ship_index()
}

/// The ship a route key should act on for a given selection: the selected ship itself, or -
/// when it sails in a convoy - the convoy's LEAD ship.
///
/// Every route key needs this, because a convoy keeps its whole route on the lead ship
/// (`ship+0x132`; measured, the other members read `0xFFFF`) while the ship panel reports a
/// member. Selecting the convoy *as a whole* reports a member too, so without this a route
/// key on a convoy reads an empty route and refuses. Acting on the leader is also what the
/// game itself does: it attaches a route loaded there to the whole convoy, and it names the
/// convoy after its leader, so the generated `-LueRosSte` name lands where it belongs.
///
/// Conservative by construction - it falls back to the selection whenever anything fails to
/// line up. `ship+0x8` is `0xFFFF` for a ship sailing alone (the game writes that into
/// `+0x6`/`+0x8` at `0x004E13F2`), which `get_convoy` rejects on its bounds check; and the
/// leader is only accepted if it names this same convoy back, so a member that has just left
/// one cannot redirect a key onto a ship the player did not select.
unsafe fn route_ship_index(selected: u16) -> u16 {
    let ships = p3_api::ships::ShipsPtr::new();
    let Some(ship) = ships.get_ship(selected) else { return selected };
    let convoy_index = ship.get_convoy_id();
    let Some(convoy) = ships.get_convoy(convoy_index) else { return selected };
    let lead = convoy.get_lead_ship_index();
    if lead == selected {
        return selected;
    }
    let Some(lead_ship) = ships.get_ship(lead) else {
        error!("convoy {convoy_index} names lead ship {lead}, which is out of range - acting on the selected ship {selected}");
        return selected;
    };
    if lead_ship.get_convoy_id() != convoy_index {
        error!(
            "convoy {convoy_index} names lead ship {lead}, but that ship is in convoy {} - acting on the selected ship {selected}",
            lead_ship.get_convoy_id()
        );
        return selected;
    }
    info!(
        "selection is ship {selected} {:?} of convoy {convoy_index}; acting on its lead ship {lead} {:?}, which carries the route",
        ship.get_name(),
        lead_ship.get_name()
    );
    lead
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
    let Some(selected) = selected_ship_index() else {
        notify("Clear route: no ship selected");
        return;
    };
    // A convoy's route lives on its lead ship, so clear it there - see [route_ship_index].
    let ship_index = route_ship_index(selected);
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

    // The route keys rename the ship after its towns, so a cleared ship would keep a name
    // describing a route it no longer has. Put it back on a pool name, the way a
    // newly built ship gets one.
    let renamed = match random_ship_name() {
        Some(pool_name) => {
            rename_ship(ship_index, &pool_name);
            format!(", renamed {pool_name}")
        }
        None => String::new(),
    };
    notify(&format!("Route cleared: {} stops removed from {name}{renamed}", previous.len()));
}

/// A name drawn from the game's own ship-name pool (`scripts/NamenSchiffe_eng.txt`), the
/// list a newly built ship is named from.
///
/// The game picks its index by stepping a shared counter (`state = (state + step) % 307`
/// at `0x0050E1A6`, rejecting values past the pool count). This deliberately does **not**
/// reuse that: the state lives on a game object and advancing it would perturb the
/// sequence the game's own naming draws from. A local mix of the game clock and a call
/// counter is enough - the only requirement is that pressing DEL twice does not hand out
/// the same name twice in the same tick.
unsafe fn random_ship_name() -> Option<String> {
    let count = p3_api::names::ship_name_count();
    if count == 0 {
        warn!("clear route: the ship-name pool is empty - leaving the name alone");
        return None;
    }
    let nth = NAME_PICKS.fetch_add(1, Ordering::Relaxed);
    let mut x = GAME_WORLD_PTR.get_game_time_raw() ^ nth.wrapping_mul(0x9E37_79B9);
    x ^= x >> 16;
    x = x.wrapping_mul(0x7FEB_352D);
    x ^= x >> 15;
    let name = p3_api::names::get_ship_name((x % count as u32) as u16)?;
    Some(name.iter().map(|&b| b as char).collect())
}

/// Counts calls to [random_ship_name] so two presses inside one game tick differ.
static NAME_PICKS: AtomicU32 = AtomicU32::new(0);

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
/// sell everything else (at STOP_SELL_LEVEL), MAX amounts - the same shape as a stop of
/// the F4 trade template, but written in place and preserving every existing instruction,
/// office transfers included. With `skip_no_buy_wares` the [NO_BUY_WARES] are not bought
/// where the town produces them - they get a sell order like anything else, not no order.
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
        // Same rule as the F4 trade stop: buy the production minus the NO_BUY_WARES, sell
        // everything else - a produced NO_BUY_WARE included. Kept identical on purpose;
        // the two are documented as the same shape.
        let produced = production[i] > 0;
        if produced && !(skip_no_buy_wares && NO_BUY_WARES.contains(&ware_id)) {
            price[i] = -buy_price(ware_index, STOP_BUY_LEVEL);
            bought.push(format!("{ware_id:?}"));
        } else {
            if produced {
                skipped.push(format!("{ware_id:?}"));
            }
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
        format!(", selling not buying [{}]", skipped.join(", "))
    };
    info!(
        "dialog setup for the {town} stop: buying [{}]{skipped}, selling {sold}, {untouched} existing instructions untouched",
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

/// True if the player has a trading office in the town. Route stops that transfer wares
/// to or from an office are wiped at load time in towns where there is none.
unsafe fn has_player_office(town_index: u8) -> bool {
    let merchant_index = OPERATIONS_PTR.get_player_merchant_index();
    GAME_WORLD_PTR.get_office_in_of(town_index as _, merchant_index as _).is_some()
}

/// Write the stops as the route file and apply them to the ship, normalising the
/// logical-first marker onto the first stop. Returns false and logs on failure.
///
/// Callers must have clamped to [`MAX_DISPLAYABLE_STOPS`] already - opening the auto-trade
/// window on a longer route crashes the game.
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

/// The route templates the F1/F2/F3 keys can set, from `p3_rou::builder`.
#[derive(Clone, Copy, Debug)]
enum RouteKind {
    FiveStop,
    SixStop,
    Suck,
    /// One self-contained trade stop per target: buy what the town produces, sell what
    /// it does not, both at the extreme Y prices. Nothing is loaded at home and
    /// everything is unloaded there, so the route is a circuit that needs no
    /// consumption figures and no office at any target.
    Trade,
    /// A collection route for a hand-picked ware list: the wares are the buy orders the
    /// player left on the ship's own FIRST stop, and the targets are every town that
    /// produces at least one of them (with ALT, every town at all), visited in the
    /// shortest closed tour. Every stop buys the whole list at the prices that first stop
    /// carries, copied through as-is - the only template that does not price itself.
    ///
    /// The only template whose ware list is explicit rather than derived from
    /// production or consumption, and the only one that requires an existing route -
    /// the first stop is its input.
    Fetch,
}

/// F1/F2/F3: rebuild the selected ship's route from a supply template. The ship's current
/// route provides the towns: its FIRST stop's town becomes the home town, and the
/// remaining unique towns, in order, the targets (further occurrences of the home town
/// are ignored; a ship without a route uses the merchant's home town and the open town
/// view). The generated route repeats the template's action stops once per target,
/// bracketed by a home load stop (summed quantities) and a home unload stop - except the
/// collection templates ([RouteKind::Suck] and [RouteKind::Fetch]), which load nothing and
/// so use a **single** home stop that transfers the hold into the office
/// (`builder::collecting_route`); the route loops back to it, so a trailing home stop
/// would only repeat it. With shift held, the target is
/// just the currently open town and the generated stops are APPENDED to the existing
/// route instead of replacing it.
///
/// **What ALT means depends on the template**, because each has a different filter worth
/// relaxing - in both cases it is a ware filter, never a quantity:
///
/// | Template | plain | with ALT |
/// |-|-|-|
/// | F1 5stop, F2 6stop | supply everything the target consumes | leave out the [NO_SUPPLY_WARES] |
/// | F3 collect, F4 trade | leave the [NO_BUY_WARES] in the town | buy those too |
/// | CTRL+F3 fetch | call only at towns that produce a wanted ware | call at every town |
///
/// For the supply templates, quantities are a week of each target's citizen and business
/// consumption and prices the R levels. Wares a target produces itself are not supplied to
/// it; targets without a
/// player office get a combined sell-and-buy trade stop (buying their produce at T)
/// instead of the office-reset stops. F3 builds a collection route instead: one
/// buy stop per target at the R price, skipping the NO_BUY_WARES and everything the
/// home town produces itself, behind the single home transfer stop.
///
/// [RouteKind::Fetch] (CTRL+F3) is the exception to the paragraph above: it takes neither
/// its wares nor its targets from the same places. The ware list is the buy orders the
/// player left on the ship's FIRST stop, the targets are every town that produces one of
/// them, and their order is the shortest closed tour rather than the order they were
/// discovered in - so it needs an existing route and takes no SHIFT. Its ALT is the one
/// in the table above that filters TOWNS rather than wares.
unsafe fn on_route_hotkey(kind: RouteKind, append: bool, alt: bool) {
    let Some(selected) = selected_ship_index() else {
        notify("Route: no ship selected");
        return;
    };
    // A convoy carries its route, and its name, on the lead ship - see [route_ship_index].
    let ship_index = route_ship_index(selected);
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

    // What a fetch route collects, and for how much: the buy orders the player left on the
    // ship's own first stop, wares and prices both. Resolved before the targets because it
    // decides them - and refused loudly, since this is the one input the template cannot
    // invent.
    let fetch_buys = if matches!(kind, RouteKind::Fetch) {
        let Some(first) = previous.first() else {
            notify("Fetch route: the ship has no route - it needs a first stop carrying the buy orders to collect");
            return;
        };
        let prices = fetch_buy_prices(first);
        if !prices.iter().any(|&price| price > 0) {
            notify(&format!(
                "Fetch route: the first stop ({load_town_name}) has no buy orders - mark the wares to collect there first"
            ));
            return;
        }
        Some(prices)
    } else {
        None
    };

    let targets: Vec<u8> = if let Some(buys) = &fetch_buys {
        // Every town that produces one of the wanted wares - or, with ALT, every town
        // there is - in the shortest closed tour from home. The ship's previous stops say
        // nothing here: the whole point is to discover the sources rather than list them
        // by hand.
        let towns = fetch_targets(load_town, buys, alt);
        if towns.is_empty() {
            notify(&format!(
                "Fetch route: no town other than {load_town_name} produces [{}]",
                ware_names(buys).join(", ")
            ));
            return;
        }
        order_towns_by_distance(load_town, towns)
    } else if append {
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
    // everything the home town does not produce itself, at the R price. (In trade stops,
    // sells take precedence per ware.)
    //
    // ALT widens it by including the NO_BUY_WARES - but only for the collection route,
    // whose whole job is buying. On the supply templates ALT already means the supply
    // filter, and letting it also widen the buying at office-less targets would have one
    // key freeing hold space and filling it again in the same press.
    let skip_no_buy = !(matches!(kind, RouteKind::Suck) && alt);
    let home_production = GAME_WORLD_PTR.get_town(load_town).get_production_values();
    let mut collect_buys = [0i32; 24];
    for ware_index in TRADE_WARES {
        let i = ware_index as usize;
        let ware_id = WareId::from_u16(ware_index).unwrap();
        if home_production[i] > 0 || (skip_no_buy && NO_BUY_WARES.contains(&ware_id)) {
            continue;
        }
        collect_buys[i] = buy_price(ware_index, COLLECT_BUY_LEVEL);
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
        if let Some(prices) = &fetch_buys {
            middle.push(builder::buy_stop(town_index, prices));
            described.push(town_name);
            continue;
        }
        if matches!(kind, RouteKind::Trade) {
            // ALT widens the buying here rather than narrowing the supplies: this
            // template supplies nothing to narrow.
            let (price, amount, bought, skipped, sold) = town_trade_basket(town_index, !alt);
            middle.push(builder::stop(town_index, builder::FLAG_X, price, amount));
            let left = if skipped.is_empty() {
                String::new()
            } else {
                format!(", selling not buying [{}]", skipped.join(", "))
            };
            described.push(format!("{town_name} (buying [{}]{left}, selling {sold})", bought.join(", ")));
            continue;
        }

        let (load_amount, sell_prices, supplied) = town_supply_basket(town_index, alt);
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

    // The two collection templates load nothing at home and only ever bring goods back,
    // so they use the one-stop home bracket: the leading home stop transfers the hold into
    // the office and the route loops straight back to it, making a trailing home stop
    // redundant. The supply and trade templates still need the load stop at the front.
    let template = if matches!(kind, RouteKind::Suck | RouteKind::Fetch) {
        builder::collecting_route(load_town, middle)
    } else {
        builder::bracketed_route(load_town, total_load, middle)
    };
    let stops = if append {
        let mut stops = previous;
        stops.extend(template);
        stops
    } else {
        template
    };
    // The auto-trade window has exactly 20 row widgets and no bound check, so a longer
    // route crashes the game the moment the player opens it - see MAX_DISPLAYABLE_STOPS.
    // Appending is what actually reaches this: a full "every town" fetch is well past 20.
    let mut stops = stops;
    if stops.len() > MAX_DISPLAYABLE_STOPS {
        let dropped = stops.len() - MAX_DISPLAYABLE_STOPS;
        stops.truncate(MAX_DISPLAYABLE_STOPS);
        warn!(
            "route: clamped to the auto-trade window's {MAX_DISPLAYABLE_STOPS} rows - dropped the last {dropped} stop(s)"
        );
        notify(&format!("Route clamped to {MAX_DISPLAYABLE_STOPS} stops ({dropped} dropped)"));
    }
    let stop_count = stops.len();
    let route_name = route_ship_name(&stops);

    if write_and_apply_route(ship_index, stops) {
        rename_ship(ship_index, &route_name);
        let action = if append { "appended to" } else { "set on" };
        // Interpolated from the level constants rather than spelled out, so retuning a
        // level cannot leave the report claiming the old letter.
        let verb = match (kind, alt) {
            (RouteKind::Suck, true) => format!("collect at {COLLECT_BUY_LEVEL:?} from (including the no-buy wares)"),
            (RouteKind::Suck, false) => format!("collect at {COLLECT_BUY_LEVEL:?} from"),
            (RouteKind::Trade, true) => format!("trade at {STOP_BUY_LEVEL:?}/{STOP_SELL_LEVEL:?} with (buying everything produced)"),
            (RouteKind::Trade, false) => format!("trade at {STOP_BUY_LEVEL:?}/{STOP_SELL_LEVEL:?} with"),
            // Every fetch stop buys the same list at the same prices, so the list belongs
            // in the verb rather than repeated once per town in `described`.
            (RouteKind::Fetch, every_town) => format!(
                "fetch [{}] (the first stop's own prices) from{}",
                fetch_buys.map(|buys| ware_names(&buys).join(", ")).unwrap_or_default(),
                if every_town { " every town, producer or not," } else { "" }
            ),
            (_, true) => "supply (skipping the low-value inputs)".to_string(),
            (_, false) => "supply".to_string(),
        };
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

/// The route name for a ship: a leading `-` so generated ships group together when
/// a ship list sorts by name, then the first three letters of each of the route's
/// towns, unique, in route order (e.g. -LueRosSte) - up to ten towns. Capped at 31
/// characters: the ship struct's inline name buffer at +0x160 is 32 bytes and ends
/// the 0x180-stride struct, so anything longer would spill into the next ship. The
/// prefix costs no town: 1 + 10 x 3 is exactly 31.
unsafe fn route_ship_name(stops: &[TradeRouteStop]) -> String {
    let mut route_name = String::from(ROUTE_NAME_PREFIX);
    let mut seen: Vec<u8> = Vec::new();
    for stop in stops {
        if seen.contains(&stop.town_index) {
            continue;
        }
        seen.push(stop.town_index);
        if route_name.chars().count() + 3 > 31 {
            break;
        }
        route_name.push_str(&town_code(stop.town_index).unwrap_or_default());
    }
    route_name
}

/// The marker [route_ship_name] puts at the front of a generated name. It is what
/// [resume_autotrade_after_thaw] uses to tell "a route ship this mod named" from a ship
/// the player named, so the two must agree - hence the shared constant.
const ROUTE_NAME_PREFIX: &str = "-";

/// A town's three-letter code as it appears inside a generated ship name: the first
/// three characters of the town's name (Luebeck -> `Lue`). All 24 town names of the
/// standard map are distinct in their first three characters, so a code identifies one
/// town; a map with two towns sharing a prefix would make [resume_autotrade_after_thaw]
/// treat them as one, which only ever means resuming a ship a little eagerly.
fn town_code(town_index: u8) -> Option<String> {
    let name = get_town_name(town_index)?;
    let code: String = name.chars().take(3).collect();
    (code.chars().count() == 3).then_some(code)
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

/// One target town's trade basket, the shape the F4 template repeats: buy what the town
/// produces (at [STOP_BUY_LEVEL]) and sell what it does not (at [STOP_SELL_LEVEL]), all at
/// MAX amounts - the price is the limit here, not a quantity, which is why this needs no
/// consumption reading.
/// With `skip_no_buy` the [NO_BUY_WARES] get no order where the town produces them.
/// Returns (prices, amounts, bought names, skipped names, sold count).
unsafe fn town_trade_basket(
    town_index: u8,
    skip_no_buy: bool,
) -> ([i32; 24], [i32; 24], Vec<String>, Vec<String>, u32) {
    let production = GAME_WORLD_PTR.get_town(town_index).get_production_values();
    let mut price = [0i32; 24];
    let mut amount = [0i32; 24];
    let mut bought = Vec::new();
    let mut skipped = Vec::new();
    let mut sold = 0;
    for ware_index in TRADE_WARES {
        let i = ware_index as usize;
        let ware_id = WareId::from_u16(ware_index).unwrap();
        // Buy the town's production minus the NO_BUY_WARES; sell everything else - which
        // includes a NO_BUY_WARE the town produces. A sell order costs nothing: the price
        // is a minimum, so it either trades at a price worth having or does not fire. Give
        // every ware an order rather than reason about which ones could pay off.
        let produced = production[i] > 0;
        if produced && !(skip_no_buy && NO_BUY_WARES.contains(&ware_id)) {
            price[i] = -buy_price(ware_index, STOP_BUY_LEVEL);
            bought.push(format!("{ware_id:?}"));
        } else {
            if produced {
                skipped.push(format!("{ware_id:?}"));
            }
            price[i] = sell_price(ware_index, STOP_SELL_LEVEL);
            sold += 1;
        }
        amount[i] = builder::MAX_AMOUNT;
    }
    (price, amount, bought, skipped, sold)
}

/// One target town's supply basket: a week of its citizen and business consumption in
/// raw units, rounded up to whole in-game units, with the R sell prices. Wares the town
/// produces itself are always excluded; with `skip_no_supply` the [NO_SUPPLY_WARES] are
/// too. Returns (amounts, prices, supplied ware count).
///
/// The quantity is a week of citizen **and** business consumption in both cases -
/// `skip_no_supply` narrows which wares are carried, never how much of them.
unsafe fn town_supply_basket(town_index: u8, skip_no_supply: bool) -> ([i32; 24], [i32; 24], u32) {
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
        if production[i] > 0 || (skip_no_supply && NO_SUPPLY_WARES.contains(&ware_id)) {
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
        sell_prices[i] = sell_price(ware_index, SUPPLY_SELL_LEVEL);
        supplied += 1;
    }
    (load_amount, sell_prices, supplied)
}

/// What a fetch route collects, read off the buy orders the player left on the ship's own
/// first stop: the maximum price per ware, 0 for a ware the route does not want.
///
/// A buy order is a NEGATED maximum price (see p3-rou's builder), so the sign of the price
/// both identifies a buy and carries its limit - and unlike the amount it is unambiguous,
/// because the game leaves stale positive base prices in slots that carry no instruction
/// but only ever negates a price for a buy. The prices are copied through verbatim, so the
/// first stop is the whole specification: which wares, and what each is worth paying. Set
/// them in the goods dialog (CTRL+Q..Y prices a whole stop, or edit a ware by hand).
fn fetch_buy_prices(first_stop: &TradeRouteStop) -> [i32; 24] {
    let mut prices = [0i32; 24];
    for ware_index in TRADE_WARES {
        let i = ware_index as usize;
        if first_stop.price[i] < 0 {
            prices[i] = -first_stop.price[i];
        }
    }
    prices
}

/// The wanted wares and the price each will be bought at, for reporting.
fn ware_names(buy_prices: &[i32; 24]) -> Vec<String> {
    TRADE_WARES
        .filter(|&ware_index| buy_prices[ware_index as usize] > 0)
        .map(|ware_index| format!("{:?}@{}", WareId::from_u16(ware_index).unwrap(), buy_prices[ware_index as usize]))
        .collect()
}

/// The fetch route's targets: every town other than `home` that produces at least one of
/// the wanted wares, or with `every_town` (ALT) simply every town other than `home`.
///
/// Production rather than stock, because this decides where a *standing* route calls: a
/// town that happens to be holding a ware today is not a source to build a circuit around,
/// while a producer keeps refilling between visits. What each stop then *buys* is the whole
/// wanted list either way - see `fetch_buys` in [on_route_hotkey].
///
/// ALT drops the filter for the case the filter gets wrong: a town can hold a wanted ware
/// without producing it - imports, an AI trader's dumping ground, a former producer - and
/// the player's own price limits decide whether anything is actually bought, so a wasted
/// call costs sailing time and nothing else. The price protects the money; production is
/// only a guess at where the goods will be.
unsafe fn fetch_targets(home: u8, buy_prices: &[i32; 24], every_town: bool) -> Vec<u8> {
    let mut towns = Vec::new();
    for town_index in 0..GAME_WORLD_PTR.get_towns_count() as u8 {
        if town_index == home {
            continue;
        }
        let production = GAME_WORLD_PTR.get_town(town_index).get_production_values();
        // into_iter() rather than a bare TRADE_WARES.any(..): `any` takes &mut self, and
        // calling it straight on a const would silently borrow a temporary copy.
        let produces_wanted = TRADE_WARES.into_iter().any(|ware_index| {
            let i = ware_index as usize;
            buy_prices[i] > 0 && production[i] > 0
        });
        if every_town || produces_wanted {
            towns.push(town_index);
        }
    }
    towns
}

/// Order the target towns into the shortest closed tour that leaves `home` and comes back
/// to it - the order the route's middle stops are generated in.
///
/// The distances are the game's own: [p3_api::class35::Class35Ptr::town_distance] runs the
/// pathfinder the ships themselves use, so a leg is as long as the water route really is,
/// coastlines and sea lanes included, not a straight line. Travel time divides that
/// distance by a per-SHIP speed factor - the same factor on every leg - so the shortest
/// tour is also the fastest one, whatever ship ends up running it, and the ship's type,
/// hull condition and load never enter the ordering.
///
/// Nearest neighbour from home, then 2-opt until no segment reversal improves the tour.
/// For the handful of towns a ware list produces this is optimal or within a percent of
/// it, and it costs one keypress: 24 towns is 276 router calls and a few hundred
/// reversals. If any distance is unavailable the discovery order is kept - a longer route
/// beats no route.
unsafe fn order_towns_by_distance(home: u8, towns: Vec<u8>) -> Vec<u8> {
    // One target has no order to choose and two are symmetric: home-A-B-home is the same
    // closed tour as home-B-A-home.
    if towns.len() < 3 {
        return towns;
    }

    let mut nodes = Vec::with_capacity(towns.len() + 1);
    nodes.push(home);
    nodes.extend_from_slice(&towns);
    let mut ids = Vec::with_capacity(nodes.len());
    for &town_index in &nodes {
        match GAME_WORLD_PTR.find_town_id(town_index) {
            Some(id) => ids.push(id),
            None => {
                warn!("fetch route: town {town_index} has no town id, keeping the discovery order");
                return towns;
            }
        }
    }

    // The whole distance matrix up front, since 2-opt needs any pair. The router depends
    // on nothing but the two endpoints' static coordinates, so half the matrix is enough
    // and the numbers are the same in every save.
    let n = nodes.len();
    let router = p3_api::class35::Class35Ptr::new();
    let mut distance = vec![0i32; n * n];
    for i in 0..n {
        for j in (i + 1)..n {
            let Some(d) = router.town_distance(ids[i], ids[j]) else {
                warn!(
                    "fetch route: the router found no route between towns {} and {}, keeping the discovery order",
                    nodes[i], nodes[j]
                );
                return towns;
            };
            distance[i * n + j] = d;
            distance[j * n + i] = d;
        }
    }
    let leg = |a: usize, b: usize| distance[a * n + b];
    let tour_length = |tour: &[usize]| -> i64 { (0..tour.len()).map(|i| leg(tour[i], tour[(i + 1) % tour.len()]) as i64).sum() };

    // Nearest neighbour from home.
    let mut tour = vec![0usize];
    let mut visited = vec![false; n];
    visited[0] = true;
    while tour.len() < n {
        let current = *tour.last().unwrap();
        let next = (1..n).filter(|&j| !visited[j]).min_by_key(|&j| leg(current, j)).unwrap();
        visited[next] = true;
        tour.push(next);
    }
    let greedy_length = tour_length(&tour);

    // 2-opt: reverse any stretch of the tour whose two cut legs get shorter for it. Home
    // stays pinned at position 0 - it is the route's bracket, not a free stop, and the
    // closing leg back to it is accounted for by the wrap in `after`. Every accepted
    // reversal strictly shortens an integer length, so this terminates on its own; the
    // pass cap only guarantees that a defect in the arithmetic cannot hang the game on a
    // keypress.
    for _ in 0..1000 {
        let mut improved = false;
        for i in 1..n - 1 {
            for k in (i + 1)..n {
                let (before, first, last, after) = (tour[i - 1], tour[i], tour[k], tour[(k + 1) % n]);
                if leg(before, last) + leg(first, after) < leg(before, first) + leg(last, after) {
                    tour[i..=k].reverse();
                    improved = true;
                }
            }
        }
        if !improved {
            break;
        }
    }
    let final_length = tour_length(&tour);

    let ordered: Vec<u8> = tour[1..].iter().map(|&i| nodes[i]).collect();
    let names: Vec<String> = ordered
        .iter()
        .map(|&town_index| get_town_name(town_index).unwrap_or_else(|| format!("town {town_index}")))
        .collect();
    debug!(
        "fetch route: {} targets ordered into a closed tour of {final_length} from {} (nearest neighbour alone: {greedy_length}): [{}]",
        ordered.len(),
        get_town_name(home).unwrap_or_else(|| format!("town {home}")),
        names.join(" -> ")
    );
    ordered
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
/// amount is set to [BUY_AMOUNT_LOADS] / [BUY_AMOUNT_BARRELS] only if the current amount
/// is 0.
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
                let amount = if ware_id.is_barrel_ware() { BUY_AMOUNT_BARRELS } else { BUY_AMOUNT_LOADS };
                amount.saturating_mul(ware_id.get_scaling())
            } else {
                stocks[i]
            };
            (-buy_price(ware_index, PriceLevel::Q), stock)
        } else {
            sold += 1;
            (sell_price(ware_index, PriceLevel::Q), stocks[i])
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

/// DEL in the office window: clear every auto-trade order back to nothing - no buy, no
/// sell, amount 0, and the "lock amount" checkbox unticked.
///
/// The inverse of [on_setup_hotkey], which is why it lives on the same window: setup only
/// ever fills in wares that have no order, so without a reset there is no way to undo it
/// short of twenty manual edits. A price of `0` is what the game itself means by "no
/// order" - [on_setup_hotkey] uses exactly that test to decide which wares it may touch.
///
/// This shadows the global route-clear DEL for as long as the window is open, the same way
/// the office F1 shadows the global route F1.
unsafe fn on_reset_office_hotkey() {
    let Some((office, office_index, town)) = resolve_office() else {
        return;
    };
    let town_index = UITradingOfficeWindowPtr::new().get_town_index() as u8;
    let merchant_index = OPERATIONS_PTR.get_player_merchant_index();
    let prices = office.get_administrator_trade_prices();
    let locks = office.get_administrator_trade_lock_bitmap();

    let mut cleared = 0;
    let mut unlocked = 0;
    for ware_index in TRADE_WARES {
        let ware_id = WareId::from_u16(ware_index).unwrap();
        let i = ware_index as usize;
        if prices[i] != 0 {
            cleared += 1;
        }
        if locks & (1 << i) != 0 {
            unlocked += 1;
        }
        // Executed directly rather than enqueued, so the refresh below sees the new values.
        execute_operation(&Operation::OfficeAutotradeSettingChange {
            stock: 0,
            price: 0,
            office_index: office_index as _,
            ware_id,
        });
        execute_operation(&Operation::OfficeAutotradeLockChange {
            ware_id,
            town_index: town_index as u16,
            merchant_index: merchant_index as u16,
            lock: false,
        });
    }
    info!("reset in {town}: cleared {cleared} orders, unlocked {unlocked} amounts");
    notify(&format!("Office reset in {town}: {cleared} orders cleared, {unlocked} unlocked"));
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
         sell_Q_1.40,sell_W_1.45,sell_E_1.50,sell_R_1.55,sell_T_1.60,sell_Y_1.65,\
         buy_Q_1.00,buy_W_1.05,buy_E_1.10,buy_R_1.15,buy_T_1.20,buy_Y_1.25,\
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
            sell_price(ware_index, PriceLevel::Q),
            buy_price(ware_index, PriceLevel::Q),
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

/// Module-relative offset of the only call to the unfreeze-port scheduled task
/// (`0x004E94A4`, opcode `0x35`), inside the task dispatcher.
const UNFREEZE_PORT_CALL_OFFSET: u32 = 0x000D89C8;

static UNFREEZE_HOOK_PTR: AtomicPtr<CallRel32Hook> = AtomicPtr::new(std::ptr::null_mut());

/// A port thawing: `thiscall` on the scheduled-tasks singleton, no arguments. The task
/// being executed is the one at `tasks+0x8`, which is how the handler itself finds the
/// town index - so we read it the same way rather than guessing.
///
/// The original runs first, so the frozen flag is already clear when we resume ships.
#[no_mangle]
unsafe extern "thiscall" fn unfreeze_port_hook(tasks: u32) {
    let original: extern "thiscall" fn(u32) =
        mem::transmute((*UNFREEZE_HOOK_PTR.load(Ordering::SeqCst)).old_absolute);

    let task = SCHEDULED_TASKS_PTR.get_scheduled_task(SCHEDULED_TASKS_PTR.get_earliest_task_index());
    // Only trust the town index if the task really is the one we expect: the hook fires
    // from a single call site, but the index is read out of shared mutable state.
    let town_index = (task.get_opcode() == SCHEDULED_TASK_OPCODE_UNFREEZE_PORT)
        .then(|| task.get_data_dword(0));

    original(tasks);

    match town_index {
        Some(index) if index < GAME_WORLD_PTR.get_towns_count() as u32 => {
            resume_autotrade_after_thaw(index as u8)
        }
        Some(index) => error!("thaw: task town index {index} out of range, no ships resumed"),
        None => error!("thaw: unfreeze hook fired on opcode {:#x}, no ships resumed", task.get_opcode()),
    }
}

/// Restart automatic trade on this mod's route ships that serve a town whose port has
/// just thawed.
///
/// A frozen port turns arriving ships away, and an auto-trade ship that was routed
/// through it can end up stopped - annoying to notice and to restart by hand, since
/// nothing in the game tells you which ships were affected. The route keys already stamp a
/// generated ship's route into its name ([route_ship_name]: [ROUTE_NAME_PREFIX] then one
/// [town_code] per route town), so the name is a reliable, cheap statement of "this ship
/// serves that town" - no route walking needed.
///
/// Only ever *sets* the flag, and only on the player's own ships whose generated name
/// names this town. A ship already trading is left alone, so the hook is a no-op in the
/// common case and can never stop a ship.
unsafe fn resume_autotrade_after_thaw(town_index: u8) {
    let Some(code) = town_code(town_index) else {
        error!("thaw: town {town_index} has no name, no ships resumed");
        return;
    };
    let town = get_town_name(town_index).unwrap_or_else(|| format!("town {town_index}"));
    let player = *(0x006DFC14 as *const u32) as u8;
    let ships = p3_api::ships::ShipsPtr::new();

    let mut resumed = Vec::new();
    let mut already = 0;
    for ship_index in 0..ships.get_ships_size() {
        let Some(ship) = ships.get_ship(ship_index) else { continue };
        if ship.get_merchant_index() != player {
            continue;
        }
        let name = ship.get_name();
        if !name.starts_with(ROUTE_NAME_PREFIX) || !name.contains(&code) {
            continue;
        }
        if ship.is_trade_route_active() {
            already += 1;
            continue;
        }
        OPERATIONS_PTR.enqueue_operation(Operation::SetTradeRouteActive {
            ship_index: ship_index as u32,
            active: true,
        });
        resumed.push(name);
    }

    if resumed.is_empty() {
        debug!("thaw in {town} ({code}): {already} route ships already trading, none to resume");
        return;
    }
    info!(
        "thaw in {town} ({code}): resumed automatic trade on [{}] ({already} already trading)",
        resumed.join(", ")
    );
    notify(&format!("{town} ice-free: restarted {} ship(s)", resumed.len()));
}
