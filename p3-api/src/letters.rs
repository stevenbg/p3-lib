use std::mem;

use crate::{
    data::p3_ptr::P3Pointer,
    game_world::GAME_WORLD_PTR,
    scheduled_tasks::SCHEDULED_TASKS_PTR,
};

pub const LETTERS_ADDRESS: u32 = 0x006DD730;
pub const LETTER_SIZE: u32 = 0x10;
/// The message type the tavern side room offers: a scripted letter (the create-letter
/// command turns every kind past `0x40` into this type, see the gitbook).
pub const LETTER_TYPE_TAVERN_MISSION: u8 = 0x71;
/// The scheduled-task opcode a tavern mission's task carries; the side room requires it
/// before accepting an offer (`0x005A7279`).
pub const TASK_OPCODE_TAVERN_MISSION: u16 = 0x1b;

/// The global message pool - every letter a merchant receives, chained per merchant.
#[derive(Clone, Debug, Copy)]
pub struct LettersPtr {
    pub address: u32,
}

impl Default for LettersPtr {
    fn default() -> Self {
        Self::new()
    }
}

impl LettersPtr {
    pub const fn new() -> Self {
        Self { address: LETTERS_ADDRESS }
    }

    /// How many slots the pool holds; it grows on demand. Also the invalid-index bound:
    /// a chain ends on an index at or above it.
    pub fn get_size(&self) -> u16 {
        unsafe { self.get(0x06) }
    }

    pub fn get_letter(&self, index: u16) -> Option<LetterPtr> {
        if index < self.get_size() {
            let base_address: u32 = unsafe { self.get(0x00) };
            Some(LetterPtr::new(base_address + index as u32 * LETTER_SIZE))
        } else {
            None
        }
    }

    /// The next tavern mission offer in a merchant's letter chain, starting at `index`,
    /// or `None` at the end of the chain. This is the game's own predicate
    /// (`0x004D7900`), which the tavern side room walks with: an offer is a letter of
    /// type `0x71` whose town byte is this town and whose descriptor holds a date still
    /// in the future.
    ///
    /// Continue a walk from the returned letter's `get_next_index`, the way the side
    /// room does at `0x005A72C7`.
    pub unsafe fn find_tavern_mission(&self, index: u16, town_index: u16) -> Option<u16> {
        let find: extern "thiscall" fn(u32, u16, u16) -> u16 = mem::transmute(0x004D7900);
        let found = find(self.address, index, town_index);
        if found < self.get_size() {
            Some(found)
        } else {
            None
        }
    }
}

impl P3Pointer for LettersPtr {
    fn get_address(&self) -> u32 {
        self.address
    }
}

/// One 16-byte message.
#[derive(Clone, Debug, Copy)]
pub struct LetterPtr {
    pub address: u32,
}

impl LetterPtr {
    pub fn new(address: u32) -> Self {
        Self { address }
    }

    /// Valid types are below `0x86`; a free pool slot holds `0xFF`.
    pub fn get_type(&self) -> u8 {
        unsafe { self.get(0x04) }
    }

    /// The town the letter belongs to - what the letters list draws in its town column.
    /// For scripted letters this is the low byte of an arbitrary script variable, so it
    /// is only meaningful for the kinds that pass a real town (a tavern mission offer
    /// does, which is how the side room finds it).
    pub fn get_town_index(&self) -> u8 {
        unsafe { self.get(0x05) }
    }

    /// The next letter in the owning merchant's chain, ended by an index at or above the
    /// pool size.
    pub fn get_next_index(&self) -> u16 {
        unsafe { self.get(0x06) }
    }

    /// For a scripted letter, the 16-byte descriptor: `+0x0` the text length, `+0x4` the
    /// date the letter stops counting (the side room requires it to be in the future),
    /// `+0x8` the index of the scheduled task carrying the mission, `+0xC` the slot in
    /// that task's merchant array belonging to this letter.
    pub fn get_descriptor(&self) -> u32 {
        unsafe { self.get(0x08) }
    }

    /// For a scripted letter, the formatted body. It begins with the letter's NUL
    /// terminated title - "Patrol", "Escort" - which is what the side room renders as its
    /// window title (`0x005D7FF2`), followed by the text itself.
    pub fn get_text(&self) -> u32 {
        unsafe { self.get(0x0c) }
    }

    /// The title bytes, still in the game's latin1 codepage.
    pub fn get_title_bytes(&self) -> Option<Vec<u8>> {
        let text = self.get_text();
        if !(0x0001_0000..0x7fff_0000).contains(&text) {
            return None;
        }
        let mut bytes = Vec::new();
        for offset in 0..64u32 {
            let byte = unsafe { *((text + offset) as *const u8) };
            if byte == 0 {
                break;
            }
            bytes.push(byte);
        }
        Some(bytes)
    }

    /// Is this tavern mission offer still open to `merchant_index`?
    ///
    /// The side room's second test (`0x005A7274`..`0x005A72A3`), applied after the type,
    /// town and date match. The offer's scheduled task - index in `descriptor+0x8`, and it
    /// must still carry opcode `0x1B` - points at the letter script's variable array in
    /// its `+0xC` (`+0x14` holds the variable count), and `descriptor+0xC` names the
    /// variable holding the merchant who took the offer.
    ///
    /// That variable reads `0xFFFFFFFF` while nobody has taken the offer and a merchant
    /// index afterwards, so the offer is open to a merchant when the variable is his own
    /// index or at or above the merchant count. A different merchant means he took it and
    /// the side room walks past the letter. Nothing in the field separates human players
    /// from AI merchants.
    ///
    /// The same array is what the creation site reads a duration from to set the task's
    /// due date (`0x00511527`) - they are different variables of one script, which is why
    /// the array holds a mix of towns, amounts and string pointers (a patrol offer's
    /// variable 4 held the bonus its letter text quotes).
    pub unsafe fn tavern_mission_is_open(&self, merchant_index: u16) -> bool {
        let descriptor = self.get_descriptor();
        if !(0x0001_0000..0x7fff_0000).contains(&descriptor) {
            return false;
        }
        // Bound the index ourselves: get_scheduled_task does not, and the game's own
        // accessor (0x004D8DB0) compares with `jbe`, so it lets index == capacity through.
        let task_index: u16 = *((descriptor + 0x8) as *const u16);
        let capacity: u16 = SCHEDULED_TASKS_PTR.get(0x0c);
        if task_index >= capacity {
            return false;
        }
        let task = SCHEDULED_TASKS_PTR.get_scheduled_task(task_index);
        if task.get_opcode() != TASK_OPCODE_TAVERN_MISSION {
            return false;
        }

        let variables: u32 = task.get(0x0c);
        if !(0x0001_0000..0x7fff_0000).contains(&variables) {
            return false;
        }

        // Bound the slot against the task's variable count, which the tavern interaction
        // handlers do (`0x0053C757`, `0x0053C7D9`) and the side room does not - it indexes
        // the array with the raw byte, reading up to a kilobyte past it. An unreadable
        // lock counts as open: this only decides what a page lists, and hiding a mission
        // that is really available is the worse error.
        let slot: u8 = *((descriptor + 0xc) as *const u8);
        let variable_count: u16 = task.get(0x12);
        if slot as u16 >= variable_count {
            return true;
        }
        let holder: u32 = *((variables + slot as u32 * 4) as *const u32);
        holder == merchant_index as u32 || holder >= GAME_WORLD_PTR.get_merchants_count() as u32
    }
}

impl P3Pointer for LetterPtr {
    fn get_address(&self) -> u32 {
        self.address
    }
}
