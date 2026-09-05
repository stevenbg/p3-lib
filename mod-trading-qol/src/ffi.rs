use std::sync::atomic::{AtomicPtr, AtomicU32, Ordering};
use std::{mem, panic};

use hooklet::windows::x86::{hook_call_rel32, hook_function_pointer, CallRel32Hook, FunctionPointerHook};
use log::{debug, error, info, warn};
use num_traits::FromPrimitive;
use p3_api::{
    data::enums::WareId,
    game_world::GAME_WORLD_PTR,
    hotkeys::{HotkeyHandler, HotkeysApi, MOD_ALT, MOD_CTRL, MOD_SHIFT},
    town::get_town_name,
    ui::ui_trading_office_window::UITradingOfficeWindowPtr,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{VK_DELETE, VK_F1, VK_F11, VK_F2, VK_F3, VK_F4};

use crate::prices::{buy_price, sell_price, PriceLevel};

/// F1: set every ware without an order to BUY (produced by the town) or SELL (rest),
/// at the Center price levels (buy t1, sell t0). Ctrl+F1: provision and lock the
/// celebration goods; Alt+F1: provision and lock the building materials.
const SETUP_KEY: u32 = VK_F1.0 as u32;

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
/// The trade wares (weapons are not administrator-tradeable).
pub(crate) const TRADE_WARES: std::ops::Range<u16> = 0..20;
/// The town-scene object pointer; its +0xc324 field is the current town index.
const TOWN_SCENE_PTR: *const u32 = 0x006e51ac as _;
const TOWN_SCENE_CURRENT_TOWN_OFFSET: u32 = 0xc324;
/// The window classes share their vtable layout: +0x118 is close, +0x120 is open, +0xF4 the
/// per-frame update the scene container calls while the window is visible.
const OFFICE_WINDOW_OPEN_POINTER_OFFSET: u32 = UITradingOfficeWindowPtr::VTABLE_OFFSET + 0x120;
const OFFICE_WINDOW_CLOSE_POINTER_OFFSET: u32 = UITradingOfficeWindowPtr::VTABLE_OFFSET + 0x118;
const OFFICE_WINDOW_UPDATE_POINTER_OFFSET: u32 = UITradingOfficeWindowPtr::VTABLE_OFFSET + 0xF4;
/// `+0x9C` is the draw the scene container calls each frame the window meets the dirty region.
const OFFICE_WINDOW_DRAW_POINTER_OFFSET: u32 = UITradingOfficeWindowPtr::VTABLE_OFFSET + 0x9C;

static OPEN_HOOK_PTR: AtomicPtr<FunctionPointerHook> = AtomicPtr::new(std::ptr::null_mut());
static CLOSE_HOOK_PTR: AtomicPtr<FunctionPointerHook> = AtomicPtr::new(std::ptr::null_mut());
static UPDATE_HOOK_PTR: AtomicPtr<FunctionPointerHook> = AtomicPtr::new(std::ptr::null_mut());
static DRAW_HOOK_PTR: AtomicPtr<FunctionPointerHook> = AtomicPtr::new(std::ptr::null_mut());
/// The number box class's key slot (module-relative pointer location): every focused number
/// box in the game gets its keys through it; ours acts only on the office's price boxes.
const NUMBER_WIDGET_KEY_POINTER_OFFSET: u32 = p3_api::ui::number_widget::VTABLE - 0x0040_0000 + p3_api::ui::number_widget::SLOT_KEY as u32;
static NUMBER_KEY_HOOK_PTR: AtomicPtr<FunctionPointerHook> = AtomicPtr::new(std::ptr::null_mut());
static DIALOG_CLOSE_HOOK_PTR: AtomicPtr<FunctionPointerHook> = AtomicPtr::new(std::ptr::null_mut());
static POPULATE_HOOKS: [AtomicPtr<CallRel32Hook>; 3] = [
    AtomicPtr::new(std::ptr::null_mut()),
    AtomicPtr::new(std::ptr::null_mut()),
    AtomicPtr::new(std::ptr::null_mut()),
];

/// The shared hotkey registry (hotkey_registry.dll), bound at start(); null = unavailable,
/// keys inert. All key dispatch goes through it - this mod installs no keyboard
/// hook of its own any more.
static HOTKEYS: AtomicPtr<HotkeysApi> = AtomicPtr::new(std::ptr::null_mut());

unsafe fn hotkeys() -> Option<&'static HotkeysApi> {
    HOTKEYS.load(Ordering::SeqCst).as_ref()
}

/// The three lifetime groups. Handles are 0 when unregistered; registration always
/// unregisters a stale handle first, so a double-fired open cannot orphan one.
const OWNER_GLOBAL: &std::ffi::CStr = c"trading-qol";
const OWNER_OFFICE: &std::ffi::CStr = c"trading-qol office";
const OWNER_DIALOG: &std::ffi::CStr = c"trading-qol goods dialog";

/// Session-global keys, registered once at start(): the route keys and the town
/// dump. (The F9/F10 debug probes live in mod-crash-reporter now.) The handlers
/// never swallow, exactly like the old all-seeing hook, which always fell through
/// to CallNextHookEx.
const GLOBAL_KEYS: [(u32, u32); 28] = [
    (TOWN_DUMP_KEY, 0),
    (ROUTE_TRADE_KEY, 0),
    (ROUTE_TRADE_KEY, MOD_SHIFT),
    (ROUTE_TRADE_KEY, MOD_ALT),
    (ROUTE_TRADE_KEY, MOD_ALT | MOD_SHIFT),
    // The supply templates: CTRL scales the load to the route's computed lap time
    // instead of the fixed week, and combines with SHIFT (append) and ALT (filter).
    (ROUTE_5STOP_KEY, 0),
    (ROUTE_5STOP_KEY, MOD_SHIFT),
    (ROUTE_5STOP_KEY, MOD_ALT),
    (ROUTE_5STOP_KEY, MOD_ALT | MOD_SHIFT),
    (ROUTE_5STOP_KEY, MOD_CTRL),
    (ROUTE_5STOP_KEY, MOD_CTRL | MOD_SHIFT),
    (ROUTE_5STOP_KEY, MOD_CTRL | MOD_ALT),
    (ROUTE_5STOP_KEY, MOD_CTRL | MOD_ALT | MOD_SHIFT),
    (ROUTE_6STOP_KEY, 0),
    (ROUTE_6STOP_KEY, MOD_SHIFT),
    (ROUTE_6STOP_KEY, MOD_ALT),
    (ROUTE_6STOP_KEY, MOD_ALT | MOD_SHIFT),
    (ROUTE_6STOP_KEY, MOD_CTRL),
    (ROUTE_6STOP_KEY, MOD_CTRL | MOD_SHIFT),
    (ROUTE_6STOP_KEY, MOD_CTRL | MOD_ALT),
    (ROUTE_6STOP_KEY, MOD_CTRL | MOD_ALT | MOD_SHIFT),
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
static GLOBAL_HANDLES: [AtomicU32; 28] = [const { AtomicU32::new(0) }; 28];

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
const DIALOG_KEYS: [(u32, u32); 16] = [
    (SETUP_KEY, 0),
    (SETUP_KEY, MOD_CTRL),
    (SETUP_KEY, MOD_ALT),
    (SETUP_KEY, MOD_CTRL | MOD_ALT),
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
static DIALOG_HANDLES: [AtomicU32; 16] = [const { AtomicU32::new(0) }; 16];

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

/// Re-arm the goods-dialog key group. [crate::goods_dialog::refresh_goods_dialog] calls
/// populate directly, which internally closes the dialog and so unregisters the keys
/// through the close hook - this is the post-call re-registration the hooked sites do.
pub(crate) unsafe fn reregister_dialog_keys() {
    register_group(OWNER_DIALOG, &DIALOG_KEYS, &DIALOG_HANDLES, dialog_hotkeys);
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

    // All keys go through the shared registry (hotkey_registry.dll); a missing or stale
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
    // The sync column's buttons are polled from the window's per-frame update.
    match hook_function_pointer(OFFICE_WINDOW_UPDATE_POINTER_OFFSET, office_window_update_hook as usize as u32) {
        Ok(hook) => UPDATE_HOOK_PTR.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
        Err(_) => {
            error!("failed to hook office window update");
            return 6;
        }
    }
    // The sync column's caption is text, drawn after the window's own draw.
    match hook_function_pointer(OFFICE_WINDOW_DRAW_POINTER_OFFSET, office_window_draw_hook as usize as u32) {
        Ok(hook) => DRAW_HOOK_PTR.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
        Err(_) => {
            error!("failed to hook office window draw");
            return 8;
        }
    }
    // Plain Q..Y typed into a focused price box reprice that one ware.
    match hook_function_pointer(NUMBER_WIDGET_KEY_POINTER_OFFSET, number_widget_key_hook as usize as u32) {
        Ok(hook) => NUMBER_KEY_HOOK_PTR.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
        Err(_) => {
            error!("failed to hook the number box key handler");
            return 7;
        }
    }

    // The goods-dialog keys are registered after populate (the dialog's open: the
    // panel's Goods button and the dialog's own arrows - populate's three call
    // sites) and unregistered on the dialog's close. Registration happens AFTER the
    // original returns because populate itself closes an already-open dialog first,
    // which unregisters; the post-call order makes that harmless.
    for (i, offset) in crate::goods_dialog::DIALOG_POPULATE_CALL_OFFSETS.iter().enumerate() {
        match hook_call_rel32(*offset, dialog_populate_hook as usize as u32) {
            Ok(hook) => POPULATE_HOOKS[i].store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
            Err(_) => {
                error!("failed to hook goods dialog populate call at module+{offset:#x}");
                return 4;
            }
        }
    }
    match hook_function_pointer(crate::goods_dialog::DIALOG_CLOSE_POINTER_OFFSET, dialog_close_hook as usize as u32) {
        Ok(hook) => DIALOG_CLOSE_HOOK_PTR.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
        Err(_) => {
            error!("failed to hook the goods dialog close slot");
            return 5;
        }
    }

    // A thawing port restarts automatic trade on the route ships that serve it. Not a
    // hotkey feature: it has to fire on the game's own event, whenever that happens.
    // Not fatal on failure: every hotkey feature still works without it.
    if let Err(reason) = crate::thaw::install() {
        error!("{reason} - thawed ports will not restart ships");
    }

    info!("loaded: global route templates F1 5stop / F2 6stop / F3 collect / F4 trade / CTRL+F3 fetch (shift appends, alt relaxes each template's filter), DEL, F11; office and goods-dialog F1 shadow the global F1 while open; thaw restarts route ships");
    0
}

#[no_mangle]
unsafe extern "thiscall" fn office_window_open_hook(window_address: u32) {
    let orig: extern "thiscall" fn(u32) = mem::transmute((*OPEN_HOOK_PTR.load(Ordering::SeqCst)).old_absolute);
    orig(window_address);
    register_group(OWNER_OFFICE, &OFFICE_KEYS, &OFFICE_HANDLES, office_hotkeys);
    // After the game's open, so our widgets register behind its children and draw on top.
    crate::sync::on_open(&UITradingOfficeWindowPtr { address: window_address });
}

#[no_mangle]
unsafe extern "thiscall" fn office_window_close_hook(window_address: u32) {
    let orig: extern "thiscall" fn(u32) = mem::transmute((*CLOSE_HOOK_PTR.load(Ordering::SeqCst)).old_absolute);
    orig(window_address);
    unregister_group(&OFFICE_HANDLES);
    crate::sync::on_close();
}

/// The scene container's per-frame update of the window (vtable `+0xF4`, `0x005D9500`).
#[no_mangle]
unsafe extern "thiscall" fn office_window_update_hook(window_address: u32) {
    let orig: extern "thiscall" fn(u32) = mem::transmute((*UPDATE_HOOK_PTR.load(Ordering::SeqCst)).old_absolute);
    orig(window_address);
    crate::sync::on_update(&UITradingOfficeWindowPtr { address: window_address });
}

/// The window's draw (vtable `+0x9C`, `0x005D95A0`): `thiscall(context, x, y, z)`, `ret 0x10`.
/// Ours paints after the game's, so the text lands on top of the page.
#[no_mangle]
unsafe extern "thiscall" fn office_window_draw_hook(window_address: u32, context: u32, x: i32, y: i32, z: u32) {
    let orig: extern "thiscall" fn(u32, u32, i32, i32, u32) = mem::transmute((*DRAW_HOOK_PTR.load(Ordering::SeqCst)).old_absolute);
    orig(window_address, context, x, y, z);
    crate::sync::on_draw(&UITradingOfficeWindowPtr { address: window_address });
}

/// The number box class's key handler (vtable `+0x1C`, `0x0045C300`): `thiscall(vk, repeat,
/// flags)`, reached only for the focused box. A level key on an office price box is ours and
/// never reaches the game's digit filter; everything else passes through.
#[no_mangle]
unsafe extern "thiscall" fn number_widget_key_hook(widget_address: u32, vk: u32, repeat: u32, flags: u32) {
    if crate::office::on_price_widget_key(widget_address, vk) {
        return;
    }
    let orig: extern "thiscall" fn(u32, u32, u32, u32) = mem::transmute((*NUMBER_KEY_HOOK_PTR.load(Ordering::SeqCst)).old_absolute);
    orig(widget_address, vk, repeat, flags);
}

/// Populate = the goods dialog opening (or moving to another stop). thiscall with
/// three stack arguments, `ret 0xC`; the original is [DIALOG_POPULATE] itself.
#[no_mangle]
unsafe extern "thiscall" fn dialog_populate_hook(dialog: u32, stop_index: u32, ship_index: u32, flag: u32) {
    let original: extern "thiscall" fn(u32, u32, u32, u32) = mem::transmute(crate::goods_dialog::DIALOG_POPULATE);
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
pub(crate) fn level_of(vk: u32) -> Option<PriceLevel> {
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
            crate::routes::on_route_hotkey(crate::routes::RouteKind::Trade, m & MOD_SHIFT != 0, m & MOD_ALT != 0, false);
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
        // [NO_SUPPLY_WARES]; CTRL scales the load to the route's lap time instead of a
        // week. F1 only reaches here when no office window or goods dialog is open -
        // theirs shadows it and swallows the key.
        (ROUTE_5STOP_KEY, m) => {
            crate::routes::on_route_hotkey(crate::routes::RouteKind::FiveStop, m & MOD_SHIFT != 0, m & MOD_ALT != 0, m & MOD_CTRL != 0)
        }
        (ROUTE_6STOP_KEY, m) => {
            crate::routes::on_route_hotkey(crate::routes::RouteKind::SixStop, m & MOD_SHIFT != 0, m & MOD_ALT != 0, m & MOD_CTRL != 0)
        }
        // CTRL turns F3 into the fetch template, which reads its wares off the ship - so
        // it never appends, and ALT widens its target list to every town.
        (ROUTE_SUCK_KEY, m) if m & MOD_CTRL != 0 => crate::routes::on_route_hotkey(crate::routes::RouteKind::Fetch, false, m & MOD_ALT != 0, false),
        (ROUTE_SUCK_KEY, m) => crate::routes::on_route_hotkey(crate::routes::RouteKind::Suck, m & MOD_SHIFT != 0, m & MOD_ALT != 0, false),
        // Internally guarded on the goods dialog being closed, as before.
        (CLEAR_ROUTE_KEY, 0) => crate::routes::on_clear_route_hotkey(),
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
        (CLEAR_ROUTE_KEY, 0) => crate::office::on_reset_office_hotkey(),
        (SETUP_KEY, MOD_CTRL) => crate::office::on_lock_staples_hotkey(),
        (SETUP_KEY, MOD_ALT) => crate::office::on_lock_building_materials_hotkey(),
        (SETUP_KEY, 0) => crate::office::on_setup_hotkey(),
        // Ctrl = buy prices, alt = sell prices, in the administrator view.
        _ => match level_of(vk) {
            Some(level) => {
                let (sell, buy) = if mods & MOD_CTRL != 0 { (None, Some(level)) } else { (Some(level), None) };
                crate::office::apply_prices(sell, buy);
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
    let Some((dialog, stop_index)) = crate::goods_dialog::goods_dialog_stop() else {
        return 1;
    };
    match (vk, mods) {
        // F1: fill the displayed stop's empty slots with buy/sell orders (plain
        // skips the NO_BUY_WARES, ctrl buys everything).
        // ALT+F1: load quantities from the supplied towns' current consumption (a week);
        // CTRL+ALT+F1: the same, but for the route's computed lap time in days.
        (SETUP_KEY, m) if m & MOD_ALT != 0 => {
            crate::goods_dialog::on_dialog_load_quantities_hotkey(dialog, stop_index, m & MOD_CTRL != 0)
        }
        (SETUP_KEY, m) => crate::goods_dialog::on_dialog_setup_hotkey(dialog, stop_index, m & MOD_CTRL == 0),
        _ => {
            if let Some(level) = level_of(vk) {
                let (sell, buy) = if mods & MOD_CTRL != 0 { (None, Some(level)) } else { (Some(level), None) };
                crate::goods_dialog::reprice_dialog_stop(dialog, stop_index, sell, buy);
            }
        }
    }
    1
}

/// The town whose view is open, from the town-scene object. On the world map this is the
/// town the player last visited.
pub(crate) unsafe fn current_town_index() -> Option<u8> {
    let scene = *TOWN_SCENE_PTR;
    if scene == 0 {
        return None;
    }
    Some(*((scene + TOWN_SCENE_CURRENT_TOWN_OFFSET) as *const u32) as u8)
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


