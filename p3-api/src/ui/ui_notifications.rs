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
    /// messages appear). The text is latin1 without a NUL; the game copies it into
    /// the slot's own string object (0x0042B6A0). No-op before the scrollmap exists.
    pub unsafe fn post_event(&self, text: &[u8]) {
        if self.address == 0 {
            return;
        }
        let mut buf = text.to_vec();
        buf.push(0);
        let enqueue: extern "thiscall" fn(this: u32, text: *const u8) = mem::transmute(0x0042B6A0);
        enqueue(self.address, buf.as_ptr());
    }
}
