//! The save sidecar: the locked amounts of [crate::locked_amounts] travel with the save in
//! a second file beside it, `<save>.qol` next to the game's `<save>.pat`.
//!
//! They cannot go into the save itself: the office record's arrays have spare entries, but
//! the AI, the goods dialog and the lock operation read all of them as orders, and the
//! minimum-store field has no spare bits (its compares are signed).
//!
//! Two brackets and the file names inside them, all from `p3_api::save_game`:
//! - the save handler's call (the queue drain's opcode `0xC2` case, which the Save button
//!   and the autosave both reach): the store is written out after the game has saved, to the
//!   name the handler opened its `.pat` with;
//! - the load routine's call (the dialog's start-game routine; a new game is a scenario
//!   load): the store is emptied after the game has loaded - the world is another one -
//!   and refilled from the sidecar beside the `.pat` the routine read, if it belongs to
//!   that save.
//!
//! The file is plain text: a first line naming the format, then one
//! `lock <town> <ware> <raw amount>` per locked ware. Its name is its identity - it sits
//! beside the save it belongs to - so there is no in-file check. A save with nothing locked
//! removes the sidecar. (If a check against the save were ever wanted, the save handler
//! writes the `.pat` before ours, so a hash of it could go in the header; overkill for now.)

use std::{
    collections::HashMap,
    ffi::CStr,
    fs, io, mem,
    os::raw::c_char,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicPtr, Ordering},
        Mutex,
    },
};

use hooklet::windows::x86::{hook_call_rel32, CallRel32Hook};
use log::{debug, error, info, warn};
use p3_api::{
    save_game::{LOAD_CALL_SITE, LOAD_READ_CALL_SITES, SAVE_FILE_OPEN_CALL_SITE, SAVE_HANDLER_CALL_SITE},
    ui::ui_trading_office_window::ROW_COUNT,
};

use crate::locked_amounts::{self, TownAmounts};

const SIDECAR_EXTENSION: &str = "qol";
/// The first line of the file.
const FORMAT_LINE: &str = "trading-qol locked amounts 1";

static SAVE_HOOK: AtomicPtr<CallRel32Hook> = AtomicPtr::new(std::ptr::null_mut());
static OPEN_HOOK: AtomicPtr<CallRel32Hook> = AtomicPtr::new(std::ptr::null_mut());
static LOAD_HOOK: AtomicPtr<CallRel32Hook> = AtomicPtr::new(std::ptr::null_mut());
static READ_HOOKS: [AtomicPtr<CallRel32Hook>; 2] = [AtomicPtr::new(std::ptr::null_mut()), AtomicPtr::new(std::ptr::null_mut())];

/// The `.pat` the game has just opened for writing or read, as it named it; set inside the
/// brackets, taken by them.
static SAVE_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);

pub(crate) unsafe fn install() -> Result<(), &'static str> {
    let sites: [(u32, u32, &AtomicPtr<CallRel32Hook>, &'static str); 5] = [
        (SAVE_HANDLER_CALL_SITE, save_hook as *const () as u32, &SAVE_HOOK, "the save handler's call"),
        (
            SAVE_FILE_OPEN_CALL_SITE,
            file_open_hook as *const () as u32,
            &OPEN_HOOK,
            "the save handler's file open",
        ),
        (LOAD_CALL_SITE, load_hook as *const () as u32, &LOAD_HOOK, "the load routine's call"),
        (
            LOAD_READ_CALL_SITES[0],
            read_hook as *const () as u32,
            &READ_HOOKS[0],
            "the load routine's archive read",
        ),
        (
            LOAD_READ_CALL_SITES[1],
            read_hook as *const () as u32,
            &READ_HOOKS[1],
            "the load routine's disk read",
        ),
    ];
    for (site, hook, slot, what) in sites {
        match hook_call_rel32(site - 0x0040_0000, hook) {
            Ok(installed) => slot.store(Box::into_raw(Box::new(installed)), Ordering::SeqCst),
            Err(_) => return Err(what),
        }
    }
    Ok(())
}

/// The path the game passed to a file operation, kept as the game spelled it.
unsafe fn capture(name: *const c_char) {
    if name.is_null() {
        return;
    }
    // The game's strings are Windows-1252; the save names are what the player typed into
    // the dialog, so the bytes are the characters.
    let path: String = CStr::from_ptr(name).to_bytes().iter().map(|&byte| byte as char).collect();
    *SAVE_PATH.lock().unwrap() = Some(PathBuf::from(path));
}

/// The save handler (`0x005472F0`), at its only call. The open office window's boxes go into
/// the store first, so a save taken with the administrator page open carries what the boxes
/// show; the sidecar is written once the game has written the save. The handler returns 0
/// when it has handled the files and 1 when it left the save to another player (the drain
/// then retries in a minute); a captured path means its `CFile::Open` succeeded.
unsafe extern "thiscall" fn save_hook(ops: u32, op: u32) -> u8 {
    let original: extern "thiscall" fn(u32, u32) -> u8 = mem::transmute((*SAVE_HOOK.load(Ordering::SeqCst)).old_absolute);
    *SAVE_PATH.lock().unwrap() = None;
    locked_amounts::flush_open_window();
    let deferred = original(ops, op);
    let path = SAVE_PATH.lock().unwrap().take();
    match path {
        Some(path) if deferred == 0 => write_sidecar(&path),
        _ => debug!("sidecar: the save handler wrote no file (deferred {deferred}), nothing written beside"),
    }
    deferred
}

/// `CFile::Open` of the `.pat`, inside the save handler: `thiscall(file, name, flags,
/// error*) -> BOOL`.
unsafe extern "thiscall" fn file_open_hook(file: u32, name: *const c_char, flags: u32, error: u32) -> u32 {
    let original: extern "thiscall" fn(u32, *const c_char, u32, u32) -> u32 = mem::transmute((*OPEN_HOOK.load(Ordering::SeqCst)).old_absolute);
    let opened = original(file, name, flags, error);
    if opened != 0 {
        capture(name);
    }
    opened
}

/// The load routine (`0x005476E0`), at its only call. Whatever it returns, the store is
/// emptied: the world in memory is not the one the store belonged to any more.
unsafe extern "thiscall" fn load_hook(ops: u32, name: *const c_char, expected_ticks: u32, keep_copy: u32, mode: u32) -> u8 {
    let original: extern "thiscall" fn(u32, *const c_char, u32, u32, u32) -> u8 = mem::transmute((*LOAD_HOOK.load(Ordering::SeqCst)).old_absolute);
    *SAVE_PATH.lock().unwrap() = None;
    let loaded = original(ops, name, expected_ticks, keep_copy, mode);
    locked_amounts::clear();
    if loaded != 0 {
        let path = SAVE_PATH.lock().unwrap().take();
        match path {
            Some(path) => read_sidecar(&path),
            None => debug!("sidecar: the load routine read no file, nothing to read beside"),
        }
    }
    loaded
}

/// `arc_CreateMemMapped(archive, name, &data, &size)` of the `.pat`, inside the load
/// routine - both the archive and the disk attempt. Nonzero means the file was found.
unsafe extern "C" fn read_hook(archive: u32, name: *const c_char, data: *mut u32, size: *mut u32) -> u32 {
    let original: extern "C" fn(u32, *const c_char, *mut u32, *mut u32) -> u32 = mem::transmute((*READ_HOOKS[0].load(Ordering::SeqCst)).old_absolute);
    let found = original(archive, name, data, size);
    if found != 0 {
        capture(name);
    }
    found
}

fn sidecar_of(save: &Path) -> PathBuf {
    save.with_extension(SIDECAR_EXTENSION)
}

unsafe fn write_sidecar(save: &Path) {
    let sidecar = sidecar_of(save);
    let entries = locked_amounts::snapshot();
    if entries.is_empty() {
        match fs::remove_file(&sidecar) {
            Ok(()) => info!("sidecar: nothing locked, removed {}", sidecar.display()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => debug!("sidecar: nothing locked, none beside {}", save.display()),
            Err(e) => warn!("sidecar: nothing locked, but could not remove {}: {e}", sidecar.display()),
        }
        return;
    }
    let mut text = format!("{FORMAT_LINE}\n");
    let mut locks = 0;
    for (town, amounts) in &entries {
        for (ware, &amount) in amounts.iter().enumerate() {
            if amount != 0 {
                text.push_str(&format!("lock {town} {ware} {amount}\n"));
                locks += 1;
            }
        }
    }
    match fs::write(&sidecar, text) {
        Ok(()) => info!("sidecar: wrote {locks} locked amounts in {} towns to {}", entries.len(), sidecar.display()),
        Err(e) => error!("sidecar: could not write {}: {e}", sidecar.display()),
    }
}

unsafe fn read_sidecar(save: &Path) {
    let sidecar = sidecar_of(save);
    let text = match fs::read_to_string(&sidecar) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            debug!("sidecar: none beside {}", save.display());
            return;
        }
        Err(e) => {
            warn!("sidecar: could not read {}: {e}", sidecar.display());
            return;
        }
    };
    let entries = match parse(&text) {
        Ok(entries) => entries,
        Err(what) => {
            warn!("sidecar: {} is not usable ({what}), ignored", sidecar.display());
            return;
        }
    };
    let towns = entries.len();
    let locks: usize = entries.values().map(|amounts| amounts.iter().filter(|&&a| a != 0).count()).sum();
    locked_amounts::replace(entries);
    info!("sidecar: read {locks} locked amounts in {towns} towns from {}", sidecar.display());
}

fn parse(text: &str) -> Result<HashMap<u8, TownAmounts>, String> {
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some(FORMAT_LINE) {
        return Err("the first line is not the format line".into());
    }
    let mut entries: HashMap<u8, TownAmounts> = HashMap::new();
    for (index, line) in lines.enumerate() {
        let number = index + 2;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut words = line.split_whitespace();
        match words.next() {
            // Files written by an earlier build carried ticks and player lines; ignore them.
            Some("ticks") | Some("player") => {}
            Some("lock") => {
                let mut field = |what: &str| -> Result<i64, String> {
                    words
                        .next()
                        .and_then(|word| word.parse::<i64>().ok())
                        .ok_or(format!("line {number}: bad {what}"))
                };
                let town = field("town")?;
                let ware = field("ware")?;
                let amount = field("amount")?;
                if !(0..=u8::MAX as i64).contains(&town) || !(0..ROW_COUNT as i64).contains(&ware) || i32::try_from(amount).is_err() {
                    return Err(format!("line {number}: out of range"));
                }
                entries.entry(town as u8).or_insert([0; ROW_COUNT])[ware as usize] = amount as i32;
            }
            _ => return Err(format!("line {number}: unknown entry")),
        }
    }
    Ok(entries)
}
