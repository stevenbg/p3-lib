//! The name pool at `0x006DDAA0`: the lists the game draws person, pirate and ship names
//! from, loaded from `scripts/Namen*.txt` at startup.
//!
//! Each list is three fields on the manager - a text blob, a `u16` offset table, and a
//! count - and one getter that turns an index into a `const char*` into the blob. Only the
//! ship list is wrapped here; the others are laid out the same way if they are ever needed
//! (`+0xC4`/`+0xD6` is the pirate list, read by `0x00512AE0`).

use std::mem;

/// The name manager. A static object, not a pointer.
pub const NAME_MANAGER_ADDRESS: u32 = 0x006DDAA0;

/// `manager+0xD8` - how many ship names `scripts/NamenSchiffe_eng.txt` supplied.
pub const SHIP_NAME_COUNT_ADDRESS: *const u16 = 0x006DDB78 as _;

/// `thiscall(manager, index) -> const char*` - the ship name at `index`, latin1 and
/// NUL-terminated, pointing into the manager's own blob. Returns the manager's empty
/// string for an out-of-range index, so the result is always readable.
const GET_SHIP_NAME_ADDRESS: u32 = 0x00512B20;

/// How many ship names the pool holds. Zero before the name files are loaded.
pub fn ship_name_count() -> u16 {
    unsafe { *SHIP_NAME_COUNT_ADDRESS }
}

/// The pool's ship name at `index`, as latin1 bytes without the NUL.
///
/// `None` when the index is out of range, so callers cannot silently get the empty
/// fallback the game returns.
pub fn get_ship_name(index: u16) -> Option<Vec<u8>> {
    if index >= ship_name_count() {
        return None;
    }
    unsafe {
        let get: extern "thiscall" fn(this: u32, index: u32) -> *const u8 = mem::transmute(GET_SHIP_NAME_ADDRESS);
        let name = get(NAME_MANAGER_ADDRESS, index as u32);
        if name.is_null() {
            return None;
        }
        let mut bytes = Vec::new();
        for i in 0..MAX_SHIP_NAME_LEN {
            let c = *name.add(i);
            if c == 0 {
                break;
            }
            bytes.push(c);
        }
        Some(bytes)
    }
}

/// `thiscall(manager, id) -> const char*` - a **person** name, first name or family name
/// depending on **bit 7** of the id: set picks the first-name pool (count `+0xD2`, offsets
/// `+0xC0`, blob `+0xAC`), clear the family-name pool (`+0xD0`, `+0xBC`, `+0xA8`). Both are
/// bounded by `count + 20` and fall back to the blob's own base out of range, so the result
/// is always readable. 213 call sites - it is the game's one person-name lookup.
const GET_PERSON_NAME_ADDRESS: u32 = 0x00512A60;

/// A person name out of either pool, as latin1 bytes: the id's bit 7 chooses which, exactly
/// as the game's own callers pass it. An [crate::auto_trader::AutoTraderPtr]'s `+0x2` is a
/// first-name id (bit 7 set) and its `+0x3` a family-name id.
///
/// `None` for an empty result, which is what the out-of-range fallback yields.
pub fn get_person_name(id: u8) -> Option<Vec<u8>> {
    unsafe {
        let get: extern "thiscall" fn(this: u32, id: u32) -> *const u8 = mem::transmute(GET_PERSON_NAME_ADDRESS);
        let name = get(NAME_MANAGER_ADDRESS, id as u32);
        if name.is_null() {
            return None;
        }
        let mut bytes = Vec::new();
        for i in 0..MAX_PERSON_NAME_LEN {
            let c = *name.add(i);
            if c == 0 {
                break;
            }
            bytes.push(c);
        }
        if bytes.is_empty() {
            None
        } else {
            Some(bytes)
        }
    }
}

/// `"First Family"` for a pair of name ids, or `None` when neither pool answers.
pub fn get_full_person_name(first_name_id: u8, last_name_id: u8) -> Option<String> {
    let first = get_person_name(first_name_id);
    let last = get_person_name(last_name_id);
    match (first, last) {
        (None, None) => None,
        (first, last) => {
            let mut parts = Vec::new();
            if let Some(f) = first {
                parts.push(crate::latin1_to_string(&f));
            }
            if let Some(l) = last {
                parts.push(crate::latin1_to_string(&l));
            }
            Some(parts.join(" "))
        }
    }
}

/// The pools hold short names; 32 bytes is the same ceiling the ship names use and no
/// person name in the shipped files comes close.
pub const MAX_PERSON_NAME_LEN: usize = 32;

/// The ship's inline name buffer is 32 bytes (`ship+0x160`), and the game's own naming
/// routine stops copying at that many characters (`cmp edx,0x20` at `0x0050E1F5`), so no
/// pool name is ever longer in practice.
pub const MAX_SHIP_NAME_LEN: usize = 32;

/// Where the game records which pool entry a ship was given (`ship+0x15E`, written at
/// `0x0050E1D1`). Renaming through the rename operation does not update it, exactly as a
/// player renaming a ship by hand does not.
pub const SHIP_NAME_INDEX_OFFSET: u32 = 0x15E;
