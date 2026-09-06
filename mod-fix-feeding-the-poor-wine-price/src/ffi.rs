//! Fixes the "feeding the poor" dialog pricing wine as salt.
//!
//! The dialog values a donation by walking its five edit controls (grain, meat, fish,
//! beer, wine in control order) and using the **row counter as the ware id** - both for
//! the barrels/loads scale byte (`0x005CAF83`) and for the price call (`push ebp` at
//! `0x005CAFCE`). The first four rows coincide with ware ids 0..3 by luck; the wine row
//! is priced as ware 4, **salt**. The total decides the donation's gate byte, so wine
//! counts at about an eighth of its value toward the "generous"/"extremely generous"
//! thresholds - and the extremely-generous branch is the one that grants the beggar
//! influx. Measured: 65 barrels of wine in Luebeck scored gate 22 where correct pricing
//! gives ~145.
//!
//! The fix hooks the single price call at `0x005CAFD4` and remaps ware 4 -> 7 (wine).
//! Salt is never legitimately passed at this site - the dialog's rows are wares
//! 0, 1, 2, 3 and 7 - so the remap cannot misfire, and no other caller of the price
//! routine is affected. The scale byte needs no fix: salt and wine are both barrel
//! wares, so the conversion was correct by coincidence.
//!
//! The handler (`0x004FE557`) prices the *delivered* goods itself through the ware
//! table, so the reputation credit and the warehouse deduction were always correct;
//! only the threshold total was wrong.
use std::{mem, sync::atomic::{AtomicPtr, Ordering}};

use hooklet::windows::x86::{hook_call_rel32, CallRel32Hook};
use log::{error, info};

/// The dialog's value loop's price call, `call 0x0052E1D0` at `0x005CAFD4`
/// (module-relative, as `hook_call_rel32` takes it).
const PRICE_CALL_OFFSET: u32 = 0x001CAFD4;

const WARE_SALT: u32 = 4;
const WARE_WINE: u32 = 7;

static PRICE_HOOK: AtomicPtr<CallRel32Hook> = AtomicPtr::new(std::ptr::null_mut());

#[no_mangle]
pub unsafe extern "C" fn start() -> u32 {
    let _ = log::set_logger(&win_dbg_logger::DEBUGGER_LOGGER);
    log::set_max_level(log::LevelFilter::Info);

    match hook_call_rel32(PRICE_CALL_OFFSET, donation_price_hook as usize as u32) {
        Ok(hook) => PRICE_HOOK.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
        Err(_) => {
            error!("failed to hook the donation dialog's price call");
            return 1;
        }
    }

    info!("feeding-the-poor wine price fix installed at 0x005CAFD4");
    0
}

/// `0x0052E1D0(this, ware, town, raw_amount)` - the market-value routine, thiscall with
/// three stack arguments. Rows 0..3 pass through untouched; the wine row's salt id is
/// corrected before the game prices it.
#[no_mangle]
unsafe extern "thiscall" fn donation_price_hook(this: u32, ware: u32, town: u32, raw_amount: i32) -> i32 {
    let original: extern "thiscall" fn(u32, u32, u32, i32) -> i32 =
        mem::transmute((*PRICE_HOOK.load(Ordering::SeqCst)).old_absolute);
    let ware = if ware == WARE_SALT { WARE_WINE } else { ware };
    original(this, ware, town, raw_amount)
}
