# mod-high-res-1600x900

Runs the game at 1600 x 900. Replaces the resolution the game computes from its options
field with 1600 x 900 (three detours on the resolution mapping), repositions the UI
elements laid out by resolution switch, ships 1600-wide replacements for the main screen
and the full map background, and reads the loading screen's layout from `accelMap.ini`.

Install: put `high_res_1600x900.dll` into the `mods` folder (requires the modloader).

## The town view's light layers are padded

The town view keeps three screen-sized 8-bit layers (stride `width + 2`, `height + 2`
rows) and smooths them in 8 x 8 blocks (`0x0059DD20`, driven by `0x0059E6E0`). A block
row is never clipped to the layer, which is harmless when the height is a multiple of 8 -
every stock resolution's is - but at 900 the last block row covers rows 896..903 of a
902-row layer, and the overrun of the last layer leaves the allocation: an access violation
at `0x0059DDC7` on entering a town, whenever a block at the bottom edge is dirty.

`src/light_layers.rs` makes the four sites that compute `height + 2` for these layers
compute `height + 10` instead - the block allocation (`0x0059DA8C`), the layer spacing
(`0x0059DAB8`) and the two per-frame clears (`0x0059FB88`, `0x0059FBA3`) - so each layer
carries one block row of padding. Each site is byte-checked before it is written.
