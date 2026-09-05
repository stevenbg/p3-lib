use std::mem;

/// The scrollmap notification manager: the popup boxes over the map - events on the
/// top left ("Game speed: ..."), incoming-letter notices on the top right ("Trading
/// information: ..."). One object, two queues of 5 slots each (left count byte
/// +0x488, right +0x7B4); an entry displays for 0x2EE0 ticks and enqueueing into a
/// full queue silently drops the message. The letter announcer (0x004D7B10) posts
/// the right-side popups via 0x0042BB20 (taking an MFC string BY VALUE - the callee
/// releases it) followed by a 0x004237D0 refresh.
pub const STATIC_UI_NOTIFICATIONS_PTR_ADDRESS: *const u32 = 0x006CBB40 as _;

#[derive(Clone, Debug, Copy)]
pub struct UINotificationsPtr {
    address: u32,
}

impl Default for UINotificationsPtr {
    fn default() -> Self {
        Self::new()
    }
}

impl UINotificationsPtr {
    pub fn new() -> Self {
        Self {
            address: unsafe { *STATIC_UI_NOTIFICATIONS_PTR_ADDRESS },
        }
    }

    /// Post a popup on the event ticker (the top-left boxes where "Game speed:"
    /// messages appear). The text is latin1 without a NUL; the enqueue's slot copies
    /// the characters into its own string (0x0064F390 is assign-from-chars). No-op
    /// unless the ticker is ready - see [Self::can_post_event].
    ///
    /// The enqueue (0x0042B6A0) takes the game's refcounted string **by value and
    /// releases it before returning** - `InterlockedDecrement` on the dword 0xC before
    /// the character data at 0x0064F267, and a pool free if that reaches zero - exactly
    /// like its right-ticker sibling 0x0042BB20. Until 30 Aug 2026 this method passed a
    /// bare Rust byte pointer here, so **every ticker post decremented a dword 12 bytes
    /// into whichever heap block preceded the buffer**: the process-wide heap corruption
    /// hunted down in `.claude/notes/done/device-lost-crash.md` (d3d9 resource links and
    /// a CoreMessaging object were the observed victims, four fatal crashes). The text
    /// is now handed over as a real game string, constructed the way the game itself
    /// does it (nil data, then assign-from-chars), so the callee's release balances
    /// this ownership exactly and frees through the game's own pools.
    pub unsafe fn post_event(&self, text: &[u8]) {
        if !self.can_post_event() {
            return;
        }
        let mut buf = text.to_vec();
        buf.push(0);
        let assign: extern "thiscall" fn(*mut u32, *const u8) -> *mut u32 = mem::transmute(STRING_ASSIGN_FROM_CHARS);
        let mut string: u32 = *STRING_NIL_HEADER_PTR + 0xc;
        assign(&mut string, buf.as_ptr());
        let enqueue: extern "thiscall" fn(this: u32, text: u32) = mem::transmute(0x0042B6A0);
        enqueue(self.address, string);
    }

    /// Whether [Self::post_event] would reach a constructed widget.
    ///
    /// A non-null manager is **not** enough: in the main menu, before a save is loaded,
    /// the manager exists but its ticker widgets do not, and posting crashes the game
    /// with a null dereference (measured 28 Aug 2026 - `0x00420FD9`,
    /// `mov eax,[ecx+0x94] / mov edx,[eax+0x14]` with `+0x94` null, reached from a debug
    /// probe pressed at the main menu).
    ///
    /// The enqueue at `0x0042B6A0` picks slot `[this+0x488]` of a 5-slot ring of
    /// `0xA0`-byte widgets at `this+0x168`, then makes a virtual call on it
    /// (`call [esi+0x58]` at `0x0042B720`, `this` = the slot). So the readiness test is
    /// that slot's vtable and the `+0x94` member the callee dereferences unguarded.
    ///
    /// A full queue returns false too, which changes nothing: the game's own
    /// `cmp al,5 / jae` at `0x0042B6CB` drops the message in that case anyway.
    pub unsafe fn can_post_event(&self) -> bool {
        if self.address == 0 {
            return false;
        }
        let count = *((self.address + EVENT_QUEUE_COUNT) as *const u8);
        if count >= EVENT_QUEUE_CAPACITY {
            return false;
        }
        let slot = self.address + EVENT_QUEUE_SLOTS + count as u32 * EVENT_QUEUE_SLOT_SIZE;
        *(slot as *const u32) != 0 && *((slot + EVENT_SLOT_READY) as *const u32) != 0
    }
}

/// The left (event) queue: a ring of [EVENT_QUEUE_CAPACITY] widgets of
/// [EVENT_QUEUE_SLOT_SIZE] bytes, with the live count in the byte at
/// [EVENT_QUEUE_COUNT]. Read out of the enqueue at `0x0042B6A0`.
const EVENT_QUEUE_SLOTS: u32 = 0x168;
const EVENT_QUEUE_SLOT_SIZE: u32 = 0xA0;
const EVENT_QUEUE_COUNT: u32 = 0x488;
const EVENT_QUEUE_CAPACITY: u8 = 5;
/// The slot member that stays null until the scrollmap's ticker widgets are built.
const EVENT_SLOT_READY: u32 = 0x94;

/// `thiscall(this, chars) -> this`: the game string's assign-from-C-string. Allocates
/// or reuses the refcounted block (header 0xC bytes before the data) and copies the
/// NUL-terminated characters. Same routine `mod-tavern-details` uses to build the
/// replacement letter popup.
const STRING_ASSIGN_FROM_CHARS: u32 = 0x0064F390;
/// The shared empty-string block every default-constructed game string points into
/// (data = block + 0xC); the release routine 0x0064F253 refuses to free it by address.
const STRING_NIL_HEADER_PTR: *const u32 = 0x006C7CD0 as _;
