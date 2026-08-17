use std::{
    collections::BTreeMap,
    mem, panic,
    str::FromStr,
    sync::atomic::{AtomicIsize, AtomicPtr, Ordering},
    sync::OnceLock,
};

use hooklet::windows::x86::{hook_function_pointer, FunctionPointerHook};
use log::error;
use num_traits::FromPrimitive;
use p3_api::{
    data::enums::WareId,
    game_world::GAME_WORLD_PTR,
    operation::Operation,
    operations::{execute_operation, OPERATIONS_PTR},
    town::get_town_name,
    ui::ui_trading_office_window::UITradingOfficeWindowPtr,
};
use serde::Deserialize;
use windows::Win32::{
    Foundation::{LPARAM, LRESULT, WPARAM},
    System::Threading::GetCurrentThreadId,
    UI::{
        Input::KeyboardAndMouse::{VK_F1, VK_F2},
        WindowsAndMessaging::{CallNextHookEx, SetWindowsHookExW, UnhookWindowsHookEx, HHOOK, WH_KEYBOARD},
    },
};

/// Set every ware without an order to BUY if the town produces it, otherwise SELL.
const SETUP_HOTKEY: usize = VK_F1.0 as usize;
/// Re-apply the reference price to every ware according to its current direction.
const PRICES_HOTKEY: usize = VK_F2.0 as usize;
/// The administrator view of the trading office window ("Trading Office" side button, pages 0-6).
const ADMINISTRATOR_PAGE: i32 = 4;
/// Amount set for buy orders, in in-game units.
const BUY_AMOUNT: i32 = 9999;
/// The window classes share their vtable layout: +0x118 is close, +0x120 is open.
const OFFICE_WINDOW_OPEN_POINTER_OFFSET: u32 = UITradingOfficeWindowPtr::VTABLE_OFFSET + 0x120;
const OFFICE_WINDOW_CLOSE_POINTER_OFFSET: u32 = UITradingOfficeWindowPtr::VTABLE_OFFSET + 0x118;

static OPEN_HOOK_PTR: AtomicPtr<FunctionPointerHook> = AtomicPtr::new(std::ptr::null_mut());
static CLOSE_HOOK_PTR: AtomicPtr<FunctionPointerHook> = AtomicPtr::new(std::ptr::null_mut());
/// Raw HHOOK of the keyboard hook; 0 while the office window is closed.
static KEYBOARD_HOOK: AtomicIsize = AtomicIsize::new(0);
/// Per-ware (buy_price, sell_price) from the reference table.
static REFERENCE: OnceLock<BTreeMap<u16, (i32, i32)>> = OnceLock::new();

#[derive(Deserialize)]
struct ReferenceTable {
    #[allow(dead_code)]
    citizens: u32,
    goods: BTreeMap<String, ReferenceGood>,
}

#[derive(Deserialize)]
struct ReferenceGood {
    #[allow(dead_code)]
    supply: Option<i32>,
    #[allow(dead_code)]
    supply_price: i32,
    buy_price: i32,
    sell_price: i32,
}

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

fn reference() -> &'static BTreeMap<u16, (i32, i32)> {
    REFERENCE.get_or_init(|| {
        let table: ReferenceTable = match toml::from_str(include_str!("reference.toml")) {
            Ok(table) => table,
            Err(e) => {
                ods(&format!("failed to parse the reference table: {e}"));
                return BTreeMap::new();
            }
        };
        let mut prices = BTreeMap::new();
        for (name, good) in table.goods {
            match WareId::from_str(&name) {
                Ok(ware_id) => {
                    prices.insert(ware_id as u16, (good.buy_price, good.sell_price));
                }
                Err(_) => ods(&format!("unknown ware {name:?} in the reference table")),
            }
        }
        prices
    })
}

#[no_mangle]
pub unsafe extern "C" fn start() -> u32 {
    let _ = log::set_logger(&win_dbg_logger::DEBUGGER_LOGGER);
    log::set_max_level(log::LevelFilter::Trace);

    panic::set_hook(Box::new(|p| {
        error!("{p}");
    }));

    fake_being_debugged();

    match hook_function_pointer(OFFICE_WINDOW_OPEN_POINTER_OFFSET, office_window_open_hook as usize as u32) {
        Ok(hook) => {
            OPEN_HOOK_PTR.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst);
        }
        Err(_) => {
            ods("failed to hook office window open");
            return 1;
        }
    }

    match hook_function_pointer(OFFICE_WINDOW_CLOSE_POINTER_OFFSET, office_window_close_hook as usize as u32) {
        Ok(hook) => {
            CLOSE_HOOK_PTR.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst);
        }
        Err(_) => {
            ods("failed to hook office window close");
            return 2;
        }
    }

    ods("loaded, F1 sets up orders by town production, F2 applies prices (administrator view only)");
    0
}

#[no_mangle]
unsafe extern "thiscall" fn office_window_open_hook(window_address: u32) {
    let orig_address = (*OPEN_HOOK_PTR.load(Ordering::SeqCst)).old_absolute;
    let orig: extern "thiscall" fn(window_address: u32) = mem::transmute(orig_address);
    orig(window_address);

    // The open hook runs on the game's main thread, which also pumps the messages, so
    // the thread-scoped keyboard hook fires on key events at any game speed.
    if KEYBOARD_HOOK.load(Ordering::SeqCst) == 0 {
        match SetWindowsHookExW(WH_KEYBOARD, Some(keyboard_hook), None, GetCurrentThreadId()) {
            Ok(hook) => KEYBOARD_HOOK.store(hook.0, Ordering::SeqCst),
            Err(_) => ods("installing the keyboard hook failed"),
        }
    }
}

#[no_mangle]
unsafe extern "thiscall" fn office_window_close_hook(window_address: u32) {
    let orig_address = (*CLOSE_HOOK_PTR.load(Ordering::SeqCst)).old_absolute;
    let orig: extern "thiscall" fn(window_address: u32) = mem::transmute(orig_address);
    orig(window_address);

    let hook = KEYBOARD_HOOK.swap(0, Ordering::SeqCst);
    if hook != 0 {
        let _ = UnhookWindowsHookEx(HHOOK(hook));
    }
}

unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 && (wparam.0 == SETUP_HOTKEY || wparam.0 == PRICES_HOTKEY) {
        let flags = lparam.0 as u32;
        // Bit 31: transition state (0 = key pressed); bit 30: previous state (0 = was up).
        // Together: fire once on the initial key-down, not on autorepeat or release.
        if flags & 0xC000_0000 == 0 {
            if wparam.0 == SETUP_HOTKEY {
                on_setup_hotkey();
            } else {
                on_prices_hotkey();
            }
        }
    }
    CallNextHookEx(HHOOK::default(), code, wparam, lparam)
}

/// Returns the administrator view's office and its index, or logs why not.
unsafe fn resolve_office() -> Option<(p3_api::data::office::OfficePtr, u16, String)> {
    let window = UITradingOfficeWindowPtr::new();
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

/// F1: for every reference ware whose current order is "do nothing", set BUY (reference
/// buy price, amount 9999) if the town produces the ware, otherwise SELL (reference
/// sell price, amount kept). Wares that already have an order are left untouched.
unsafe fn on_setup_hotkey() {
    let Some((office, office_index, town)) = resolve_office() else {
        return;
    };
    let window = UITradingOfficeWindowPtr::new();
    let production = GAME_WORLD_PTR.get_town(window.get_town_index() as u8).get_production_values();
    let prices = office.get_administrator_trade_prices();
    let stocks = office.get_administrator_trade_stock();

    let mut bought = Vec::new();
    let mut sold = 0;
    let mut untouched = 0;
    let mut new_stocks = stocks;
    for (&ware_index, &(buy_price, sell_price)) in reference() {
        let ware_id = WareId::from_u16(ware_index).unwrap();
        let i = ware_index as usize;
        if prices[i] != 0 {
            untouched += 1;
            continue;
        }
        let produced = production[i] > 0;
        let price = if produced {
            bought.push(format!("{ware_id:?}"));
            new_stocks[i] = BUY_AMOUNT.saturating_mul(ware_id.get_scaling());
            -buy_price
        } else {
            sold += 1;
            sell_price
        };
        // Executed directly (not enqueued) so the view refresh below sees the new values.
        execute_operation(&Operation::OfficeAutotradeSettingChange {
            stock: new_stocks[i],
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

/// Re-selects the administrator page, which rebuilds the direction arrows and prices.
/// The displayed amounts are cached in widget objects that only a full window reopen
/// rebuilds - known cosmetic limitation, the office data itself is correct.
unsafe fn refresh_administrator_view() {
    UITradingOfficeWindowPtr::new().select_new_page(ADMINISTRATOR_PAGE);
}

/// F2: re-apply the reference price to every ware that has an order, keeping its
/// direction and amount: buys get the buy price, sells get the sell price.
unsafe fn on_prices_hotkey() {
    let Some((office, office_index, town)) = resolve_office() else {
        return;
    };
    let prices = office.get_administrator_trade_prices();
    let stocks = office.get_administrator_trade_stock();

    let mut updated = 0;
    for (&ware_index, &(buy_price, sell_price)) in reference() {
        let i = ware_index as usize;
        let price = match prices[i] {
            0 => continue,
            p if p < 0 => -buy_price,
            _ => sell_price,
        };
        if price == prices[i] {
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
    ods(&format!("prices: updated {updated} wares to reference prices in {town}"));
    refresh_administrator_view();
}
