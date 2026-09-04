//! Pad the town view's light layers by one block row.
//!
//! The town view keeps three screen-sized 8-bit layers (object with vtable `0x00677BE4`:
//! `+0x4` width, `+0x8` height, stride `width + 2`), allocated as one block of
//! `3 * ((w + 2) * (h + 2) + 1) + w` bytes at `0x0059DA9E` and spaced `(w + 2) * (h + 2)`
//! apart (`0x0059DABE`). A smoothing pass works on them in 8x8 blocks: `0x0059DD20(bx, by)`
//! writes eight rows starting at `by * 8`, and the block list it serves (`0x0059E6E0`) is
//! filled from pixel positions without clipping the last row. For a height that is a
//! multiple of 8 the last block ends exactly on the layer's border row; for 900 the block
//! row 112 covers rows 896..903 while the layer has 902, so the two extra rows of the last
//! layer land past the allocation - an access violation on entering a town whenever a block
//! at the bottom edge is dirty (dump of 5 Sep 2026: fault `0x0059DDC7`, layer `+0x1C`,
//! offset `0x161686` = row 903).
//!
//! Four sites compute `height + 2` for these layers; making every one of them `height + 10`
//! gives each layer eight rows of padding and keeps them spaced consistently. The per-frame
//! clears (`0x0059FB82`) use two of them, the allocation and the spacing the other two.
use log::{error, info};
use p3_api::memory::write_readonly;

/// The immediate byte in each `add reg, 2` / `lea reg, [reg + 2]` that adds the border
/// rows to the height, with the full instruction as the guard.
const SITES: [(u32, [u8; 3]); 4] = [
    (0x0059_DA8C, [0x83, 0xc3, 0x02]), // add ebx, 2   - block allocation size
    (0x0059_DAB8, [0x8d, 0x48, 0x02]), // lea ecx, [eax+2] - layer spacing
    (0x0059_FB88, [0x83, 0xc2, 0x02]), // add edx, 2   - clear of layer +0x18
    (0x0059_FBA3, [0x83, 0xc2, 0x02]), // add edx, 2   - clear of layer +0x1C
];
/// One 8-row block of padding on top of the two border rows.
const PADDED_BORDER_ROWS: u8 = 2 + 8;

pub(crate) unsafe fn install() -> Result<(), &'static str> {
    for (address, expected) in SITES {
        let actual = std::slice::from_raw_parts(address as *const u8, 3);
        if actual != expected {
            error!("light layers: unexpected bytes at {address:#010x}: {actual:02x?}, expected {expected:02x?}");
            return Err("the town view's light layer size sites");
        }
    }
    for (address, _) in SITES {
        write_readonly(address + 2, &[PADDED_BORDER_ROWS])?;
    }
    info!("light layers: padded by one block row at {} sites", SITES.len());
    Ok(())
}
