use std::{
    mem, panic,
    sync::atomic::{AtomicIsize, AtomicPtr, Ordering},
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
use windows::Win32::{
    Foundation::{LPARAM, LRESULT, WPARAM},
    System::Threading::GetCurrentThreadId,
    UI::{
        Input::KeyboardAndMouse::{GetKeyState, VIRTUAL_KEY, VK_CONTROL, VK_F1, VK_F11, VK_F2, VK_F3, VK_F4, VK_MENU, VK_SHIFT},
        WindowsAndMessaging::{CallNextHookEx, SetWindowsHookExW, UnhookWindowsHookEx, HHOOK, WH_KEYBOARD},
    },
};

use crate::prices::{buy_price, sell_price, BuyLevel, SellLevel};

/// F1: set every ware without an order to BUY (produced by the town) or SELL (rest),
/// at the F2 price levels.
const SETUP_KEY: usize = VK_F1.0 as usize;
/// F2: set both directions' prices: sells at t0 (the supply price), buys at t1.
const BOTH_PRICES_KEY: usize = VK_F2.0 as usize;
/// F3: set only buy prices. Plain: t1; ctrl: halfway t0..t1; alt: halfway t1..t2; shift: t2.
const BUY_PRICES_KEY: usize = VK_F3.0 as usize;
/// F4: set only sell prices. Plain: t0; ctrl: halfway 0..t0; alt: halfway t0..t1; shift: t1.
const SELL_PRICES_KEY: usize = VK_F4.0 as usize;
/// F11: dump thresholds, base prices and the price levels to the log and a CSV.
const DEBUG_KEY: usize = VK_F11.0 as usize;
/// Amount set for buy orders whose current amount is 0, in in-game units.
const BUY_AMOUNT: i32 = 9999;
/// The administrator view of the trading office window ("Trading Office" side button, pages 0-6).
const ADMINISTRATOR_PAGE: i32 = 4;
/// The trade wares (weapons are not administrator-tradeable).
const TRADE_WARES: std::ops::Range<u16> = 0..20;
/// The window classes share their vtable layout: +0x118 is close, +0x120 is open.
const OFFICE_WINDOW_OPEN_POINTER_OFFSET: u32 = UITradingOfficeWindowPtr::VTABLE_OFFSET + 0x120;
const OFFICE_WINDOW_CLOSE_POINTER_OFFSET: u32 = UITradingOfficeWindowPtr::VTABLE_OFFSET + 0x118;

static OPEN_HOOK_PTR: AtomicPtr<FunctionPointerHook> = AtomicPtr::new(std::ptr::null_mut());
static CLOSE_HOOK_PTR: AtomicPtr<FunctionPointerHook> = AtomicPtr::new(std::ptr::null_mut());
/// Raw HHOOK of the keyboard hook; 0 while the office window is closed.
static KEYBOARD_HOOK: AtomicIsize = AtomicIsize::new(0);

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

    ods("loaded, F1 setup, F2 sell+buy prices, F3 buy prices, F4 sell prices (with ctrl/alt/shift levels), F11 debug");
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
            match wparam.0 {
                w if w == SETUP_KEY => on_setup_hotkey(),
                w if w == BOTH_PRICES_KEY => apply_prices(Some(SellLevel::AtT0), Some(BuyLevel::AtT1)),
                w if w == BUY_PRICES_KEY => {
                    let level = if ctrl {
                        BuyLevel::MidT0T1
                    } else if alt {
                        BuyLevel::MidT1T2
                    } else if shift {
                        BuyLevel::AtT2
                    } else {
                        BuyLevel::AtT1
                    };
                    apply_prices(None, Some(level));
                }
                w if w == SELL_PRICES_KEY => {
                    let level = if ctrl {
                        SellLevel::MidZeroT0
                    } else if alt {
                        SellLevel::MidT0T1
                    } else if shift {
                        SellLevel::AtT1
                    } else {
                        SellLevel::AtT0
                    };
                    apply_prices(Some(level), None);
                }
                w if w == DEBUG_KEY => on_debug_hotkey(),
                _ => {}
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

/// F1: for every ware whose current order is "do nothing", set BUY (at the F2 buy
/// level) if the town produces the ware, otherwise SELL (at the F2 sell level). Wares
/// that already have an order are left untouched; a buy's amount is set to BUY_AMOUNT
/// only if the current amount is 0.
unsafe fn on_setup_hotkey() {
    let Some((office, office_index, town)) = resolve_office() else {
        return;
    };
    let town_index = UITradingOfficeWindowPtr::new().get_town_index() as u8;
    let production = GAME_WORLD_PTR.get_town(town_index).get_production_values();
    let thresholds = GAME_WORLD_PTR.get_town(town_index).get_price_thresholds();
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
            (-buy_price(ware_index, &thresholds, BuyLevel::AtT1), stock)
        } else {
            sold += 1;
            (sell_price(ware_index, &thresholds, SellLevel::AtT0), stocks[i])
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
unsafe fn apply_prices(sell: Option<SellLevel>, buy: Option<BuyLevel>) {
    let Some((office, office_index, town)) = resolve_office() else {
        return;
    };
    let thresholds = GAME_WORLD_PTR
        .get_town(UITradingOfficeWindowPtr::new().get_town_index() as u8)
        .get_price_thresholds();
    let current_prices = office.get_administrator_trade_prices();
    let stocks = office.get_administrator_trade_stock();

    let mut updated = 0;
    for ware_index in TRADE_WARES {
        let i = ware_index as usize;
        let price = match current_prices[i] {
            0 => continue,
            p if p < 0 => match buy {
                Some(level) => -buy_price(ware_index, &thresholds, level),
                None => continue,
            },
            _ => match sell {
                Some(level) => sell_price(ware_index, &thresholds, level),
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
unsafe fn on_debug_hotkey() {
    let window = UITradingOfficeWindowPtr::new();
    let town_index = window.get_town_index();
    let town_name = get_town_name(town_index as u8).unwrap_or_else(|| "<unknown>".into());
    let thresholds = GAME_WORLD_PTR.get_town(town_index as u8).get_price_thresholds();
    let production = GAME_WORLD_PTR.get_town(town_index as u8).get_production_values();

    ods(&format!("debug dump for {town_name} (difficulty constant {}):", crate::prices::DIFFICULTY_D));
    let mut csv = String::from(
        "ware,base/unit,sell@t0,sell@mid_0_t0,sell@mid_t0_t1,sell@t1,buy@mid_t0_t1,buy@2weeks,buy@t1,buy@mid_t1_t2,buy@t2,prod/day,(t2-t1)/10,t0 units,t1 units,t2 units,t3 units\n",
    );
    for ware_index in TRADE_WARES {
        let i = ware_index as usize;
        let ware_id = WareId::from_u16(ware_index).unwrap();
        let scaling = ware_id.get_scaling();
        let base_per_unit = crate::prices::base_price_per_unit(ware_index);
        let [t0, t1, t2, t3] = thresholds[i];
        ods(&format!(
            "{ware_id:?}: t=[{t0}, {t1}, {t2}, {t3}] raw ({} units of week supply), base {base_per_unit:.1}/unit, sell {}, buy {}",
            t0 / scaling,
            sell_price(ware_index, &thresholds, SellLevel::AtT0),
            buy_price(ware_index, &thresholds, BuyLevel::AtT1),
        ));
        csv.push_str(&format!(
            "{ware_id:?},{base_per_unit:.1},{},{},{},{},{},{},{},{},{},{:.1},{:.1},{},{},{},{}\n",
            sell_price(ware_index, &thresholds, SellLevel::AtT0),
            sell_price(ware_index, &thresholds, SellLevel::MidZeroT0),
            sell_price(ware_index, &thresholds, SellLevel::MidT0T1),
            sell_price(ware_index, &thresholds, SellLevel::AtT1),
            buy_price(ware_index, &thresholds, BuyLevel::MidT0T1),
            buy_price(ware_index, &thresholds, BuyLevel::TwoWeeksSupply),
            buy_price(ware_index, &thresholds, BuyLevel::AtT1),
            buy_price(ware_index, &thresholds, BuyLevel::MidT1T2),
            buy_price(ware_index, &thresholds, BuyLevel::AtT2),
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
