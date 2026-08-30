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

/// The ship/route status notes filed through the creator `0x00548CA0` - a thiscall on
/// the operations object with six stack args `(merchant, 0, 0, town, subtype,
/// ship_index)` (`ret 0x18`). The renderer (`0x005483F6`) dispatches the letter's
/// subtype (0..0x2A) through the case map at `0x00548834` and the jump table at
/// `0x005487E8`; the trade-route texts ("%s's trade route: ...") live in the pointer
/// table at `0x006B04F4` (indexed 6..11). Subtype 5 is the "has docked in %s" note;
/// subtypes 0x24..0x28 are the route-stopped notes, 0x28 being "crew number too low".
///
/// Subtype 0x28 has exactly one creation site, `0x00518AB2`, guarded by the game's
/// actual departure check at `0x00518A96`: `cmp [ship+0x40], MIN_SAILORS[type]` - crew
/// below the `0x673660` table files 0x28, at-or-above files the generic subtype 0
/// instead (`0x00518ABD`). mod-auto-supply hooks that site to hire the shortfall.
pub const ROUTE_NOTE_KIND_CREW_TOO_LOW: u32 = 0x28;
/// `0x00518AB2`, module-relative for `hook_call_rel32`.
pub const ROUTE_NOTE_CREW_TOO_LOW_CALL_SITE_OFFSET: u32 = 0x00118AB2;

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

    /// The sum a tavern mission offer is worth, or `None` for a mission that never states
    /// one.
    ///
    /// The missions are script files rather than code - `missions_addon/*.p2m` inside
    /// `p2arch0_eng.cpr`, one per script id as `scripts/missions_eng.ini` names them - in
    /// the same bytecode the interpreter at `0x004ECF64` runs, so each mission's own
    /// arithmetic can be read off the file:
    ///
    /// |Script|File|Mission|What it pays|
    /// |-|-|-|-|
    /// |8|`Schmuggler`|smuggler|`loads * 150` on delivery|
    /// |9|`TransportAuftrag`|trader|`loads * 90` on delivery|
    /// |11|`PiratVernichten`|pirate hunter|variable 5, `(rand%3 + 1) * 1500 + rand%10 * 100`|
    /// |15|`Eskorte`|escort, fugitive|variable 13, `rand%20 * 100 + 3000`|
    /// |16|`patrouille`|patrol|variable 4, `(rand%3 + 1) * 1700` per foiled ambush|
    ///
    /// Scripts 11, 15 and 16 compute their sum before they send the offer, so a pending
    /// offer already holds it, and their letters quote the same variable (the templates
    /// name their variables: `%c` followed by a variable byte).
    ///
    /// The two transport orders hold no sum at all - their letters only promise to pay
    /// well - but the rate is fixed in the script, applied to the cargo at the moment it
    /// pays (`v3 * 150` at command 85 of the smuggler, `v3 * 90` at command 59 of the
    /// trader), so the figure here is derived from the cargo instead of read. The rate is
    /// the whole difference between the two, apart from the smuggler's risk of losing the
    /// goods, which falls as the ship carries more other cargo.
    ///
    /// The patrol's figure is a rate rather than a fee: variable 3 counts foiled ambushes,
    /// so the variable holds what one ambush is worth, and its voyage pay is never stated.
    /// The courier (script 13) pays `(days_to_spare * 220) + 50` and only works it out
    /// when the voyage ends, so it has nothing to show while it waits.
    ///
    /// Variables are reused as a script runs - a delivered order overwrites its answer
    /// slot with the sum it paid - so the table only holds for an offer still on the table.
    pub unsafe fn get_reward(&self) -> Option<u32> {
        let (script, variables, count) = self.tavern_mission_script()?;
        // The transport orders pay a rate per load; every other mission holds the finished
        // sum in a variable, which is the same thing with a rate of one.
        let (variable, rate): (u16, u32) = match script {
            8 => (3, 150),
            9 => (3, 90),
            11 => (5, 1),
            15 => (13, 1),
            16 => (4, 1),
            _ => return None,
        };
        if variable >= count {
            return None;
        }
        Some((*((variables + variable as u32 * 4) as *const u32)).wrapping_mul(rate))
    }

    /// What an offer asks the player to *pay*, rather than what it pays him.
    ///
    /// Only the treasure map (script 12, `SchatzKarte`) charges anything: it sells one
    /// piece of a map for variable 1, `(rand%7) * 100 + 800`, so 800 to 1400. That is the
    /// sum its letter quotes, computed before the offer goes out, and the sum the script
    /// takes back when the player accepts - `v0 = 0; v0 = v0 - v1; pay(merchant, v0)`,
    /// the pay command with a negative amount. What the recovered treasure is worth is a
    /// different variable (8), and the script only fills it in at the end.
    pub unsafe fn get_asking_price(&self) -> Option<u32> {
        let (script, variables, count) = self.tavern_mission_script()?;
        let variable: u16 = match script {
            12 => 1,
            _ => return None,
        };
        if variable >= count {
            return None;
        }
        Some(*((variables + variable as u32 * 4) as *const u32))
    }

    /// The offer's script id, its variable array and the variable count - the three things
    /// every per-script field lookup needs, from the mission's scheduled task.
    unsafe fn tavern_mission_script(&self) -> Option<(u16, u32, u16)> {
        let descriptor = self.get_descriptor();
        if !(0x0001_0000..0x7fff_0000).contains(&descriptor) {
            return None;
        }
        let task_index: u16 = *((descriptor + 0x8) as *const u16);
        if task_index >= SCHEDULED_TASKS_PTR.get::<u16>(0x0c) {
            return None;
        }
        let task = SCHEDULED_TASKS_PTR.get_scheduled_task(task_index);
        if task.get_opcode() != TASK_OPCODE_TAVERN_MISSION {
            return None;
        }
        let variables: u32 = task.get(0x0c);
        if !(0x0001_0000..0x7fff_0000).contains(&variables) {
            return None;
        }
        Some((task.get(0x10), variables, task.get(0x12)))
    }

    /// The cargo a tavern mission offer needs a ship for, in loads, or `None` for a
    /// mission that carries nothing.
    ///
    /// Variable 3 of the two transport scripts - trader (9) and smuggler (8), which are
    /// the same script with different rates and a different tone. Both roll it the same
    /// way, `rand() % 40` then `+ 4`, so an order asks for 4 to 43 loads; the load command
    /// later fills a ship and the script waits until the amount on board reaches this
    /// variable. (Read from the script files, `missions_addon/TransportAuftrag.p2m` and
    /// `Schmuggler.p2m`, commands 8..13 and 32..33 of both; it also matched the amount the
    /// letter quoted in all five offers seen in game.)
    ///
    /// The courier (script 13) asks for three loads in the words of its letter, as a
    /// literal `3 \L`, and keeps no variable for it, so it returns `None`.
    pub unsafe fn get_required_loads(&self) -> Option<u32> {
        let (script, variables, count) = self.tavern_mission_script()?;
        let variable = match script {
            8 | 9 => 3,
            _ => return None,
        };
        if variable >= count {
            return None;
        }
        Some(*((variables + variable as u32 * 4) as *const u32))
    }

    /// The town a transport order's cargo has to reach, or `None` for a mission that has
    /// no destination.
    ///
    /// Variable 5 of the transport scripts - trader (9) and smuggler (8) - and variable 10
    /// of the passenger script (15), which serves both the escort and the fugitive.
    ///
    /// The transport scripts pick it with the random-town command and re-roll it against
    /// the order's own town (commands 14..19 of both files), and script 15 gets it out of
    /// its route command. Each script's letter template settles it beyond doubt: a
    /// placeholder carries the variable it prints, and the trader's reads "ship my
    /// `%B[v3]` goods from `%t[v0]` to `%t[v5]`" while the escort's promises to sail to
    /// `%t[v10]`. Seven offers in game agreed (traders to Malmö, Edinburgh and Ladoga,
    /// smugglers to Edinburgh and London, an escort to Rostock and a fugitive to Riga).
    pub unsafe fn get_destination_town_index(&self) -> Option<u8> {
        let (script, variables, count) = self.tavern_mission_script()?;
        let variable = match script {
            8 | 9 => 5,
            15 => 10,
            _ => return None,
        };
        if variable >= count {
            return None;
        }
        let town = *((variables + variable as u32 * 4) as *const u32);
        if town < 0xff {
            Some(town as u8)
        } else {
            None
        }
    }

    /// Does the offer keep its destination from the player until he accepts?
    ///
    /// True for a smuggler (script 8), false for a trader (script 9). The two scripts are
    /// otherwise the same, and their letter templates say exactly where the line is: an
    /// offer's text holds the question and the reply that follows acceptance separated by
    /// a `|`, and the smuggler asks only for "a ship with `%B[v3]` of free storage space"
    /// before the bar, naming `%t[v5]` after it, while the trader offers to "ship my
    /// `%B[v3]` goods from `%t[v0]` to `%t[v5]`" before it. A display that shows only what
    /// the player could know has to respect that, even though the field is readable either
    /// way.
    pub unsafe fn tavern_mission_conceals_destination(&self) -> bool {
        matches!(self.tavern_mission_script(), Some((8, _, _)))
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
