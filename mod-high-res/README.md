# mod-high-res

Runs the game at 1920 x 1080. The game derives its resolution from an options field with a
handful of fixed mappings; the mod replaces the result with 1920 x 1080 at the three places
it is computed (on leaving the options menu, before and after a scene load), repositions
the UI elements the game lays out by resolution switch (the side panel and the top bar),
fills the bottom-right corner the wider frame leaves empty, ships a 1920-wide replacement
for the full map background, and reads the loading screen's layout from `accelMap.ini`.

Install: put `high_res.dll` into the `mods` folder (requires the modloader).

## Adapting it to another resolution: keep the height a multiple of 8

The town view keeps three screen-sized 8-bit light layers, `(width + 2) x (height + 2)`
bytes each, allocated in one block (`0x0059DA9E`) and smoothed in 8 x 8 pixel blocks
(`0x0059DD20`, driven by the dirty-block list in `0x0059E6E0`). The block row is never
clipped to the layer: the last row of blocks always writes up to row `ceil(height / 8) *
8`. With a height that is a multiple of 8 - every resolution the game ships with - that is
exactly the layer's border row and nothing happens. With any other height the last block
row runs past the layer, and for the last of the three layers past the allocation: an
access violation at `0x0059DDC7` on entering a town, whenever a block on the bottom edge
is dirty.

So a resolution whose height is not a multiple of 8 (900 and 1050 are not; 768, 1080, 1200
and 1440 are) must also pad the layers - the four sites that compute `height + 2` for them
are the allocation size (`0x0059DA8C`), the layer spacing (`0x0059DAB8`) and the two
per-frame clears (`0x0059FB88`, `0x0059FBA3`); one extra block row, `height + 10`, is
enough - or clip the block row in the driver. 1080 divides by 8, so this mod does neither.
