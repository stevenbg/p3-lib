//! Widget tooltips and the string resources behind them.
//!
//! A widget's tooltip is a numeric id per button state at `widget + 0x80` (the widget loader
//! `0x004B2080` fills it from the `ToolTip=` key of the widget's ini section). On hover a
//! widget's own `get tooltip text` vtable method (`0x004B1F80`) reads that id and resolves it
//! to a string with `0x00654C58`, which is a thin wrapper over the Win32 `LoadStringA`
//! (`0x00654CDC` -> USER32 `LoadStringA` at `[0x0066A494]`). So the text is an `RT_STRING`
//! resource compiled into `Patrician3.exe`, keyed by id.
//!
//! **Not every widget has a tooltip.** `0x004B1F80` is a vtable method the button classes
//! carry but the [number box](super::number_widget) class does not, so the scene's tooltip
//! system never asks a number box for text. The trading office's amount and price boxes show
//! no tooltip of their own; the "Quantity, to remain unsold" line belongs to the stock `+`/`-`
//! buttons beside them, and the lock checkbox is a button too (see [LOCK_CHECKBOX_TOOLTIP]).
//!
//! (Not to be confused with `widget + 0x70..+0x7c`, four per-state **hover-sound** ids that
//! the same loader also fills and the hover handler plays through the sound manager
//! `[0x006DCCDC]`, which holds 20 `.wav` entries.)
//!
//! **Editing a tooltip in place.** `RT_STRING` resources are UTF-16LE, stored in blocks of 16
//! strings, each a 16-bit length word then that many code units. Overwriting the code units
//! of one string, without touching its length word or its neighbours, changes what
//! `LoadStringA` returns for that id - a targeted patch, no hook of the game-wide loader. The
//! slot's length is fixed: a replacement must fit, and a shorter one ends with a NUL so the
//! game's C-string handling stops there (the extra units are zeroed).

use crate::memory::write_readonly;

/// The trading office lock checkbox's tooltip: `RT_STRING` id 20059. Its code units start at
/// this address (the length word is the `u16` before it), and the slot holds this many UTF-16
/// code units. Extracted from the executable's resource directory, not typed.
pub const LOCK_CHECKBOX_TOOLTIP: StringResource = StringResource {
    text_address: 0x006F_59DE,
    capacity: 45,
    original: "Lock min. store quantity for auto trade ships",
};

/// One `RT_STRING` slot in the loaded image: where its UTF-16 code units are, how many fit,
/// and what it holds in the stock game so an overwrite can verify the build before writing.
pub struct StringResource {
    /// The address of the first UTF-16 code unit (the length word is the `u16` before it).
    pub text_address: u32,
    /// How many UTF-16 code units the slot holds - the fixed length a replacement must fit.
    pub capacity: usize,
    /// The text the stock executable has here.
    pub original: &'static str,
}

impl StringResource {
    /// Replace the resource's text with `text` (latin1). Refuses a `text` longer than
    /// [capacity](Self::capacity), and verifies the slot still holds
    /// [original](Self::original) so a different game build fails loudly instead of writing
    /// over the wrong bytes. The length word is left as it is; `text` is followed by a NUL and
    /// the rest of the slot is zeroed, so the game shows exactly `text`.
    ///
    /// # Safety
    /// The image must be the one these offsets were read from; see [write_readonly].
    pub unsafe fn overwrite(&self, text: &str) -> Result<(), &'static str> {
        if text.chars().count() > self.capacity {
            return Err("replacement text is longer than the resource slot");
        }
        let holds_original = (0..self.original.chars().count())
            .zip(self.original.chars())
            .all(|(i, expected)| *((self.text_address + i as u32 * 2) as *const u16) == expected as u16);
        if !holds_original {
            return Err("the string resource does not hold the expected text - wrong game build");
        }
        let mut bytes = vec![0u8; self.capacity * 2];
        for (i, ch) in text.chars().enumerate() {
            let unit = ch as u16;
            bytes[i * 2] = unit as u8;
            bytes[i * 2 + 1] = (unit >> 8) as u8;
        }
        write_readonly(self.text_address, &bytes)
    }
}
