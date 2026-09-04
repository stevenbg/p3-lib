# mod-high-res-1536x864

Runs the game at 1536 x 864. Replaces the resolution the game computes from its options
field with 1536 x 864 (three detours on the resolution mapping), repositions the UI
elements laid out by resolution switch, ships replacements for the full map background and
the top bar and side panel pieces at this size, and reads the loading screen's layout from
`accelMap.ini`.

Install: put `high_res_1536x864.dll` into the `mods` folder (requires the modloader).

864 is a multiple of 8, so the town view's light layers need no padding (see
`mod-high-res`'s README for why that matters).

## Images

The three 8-bit BMPs in `src/` are produced by `resize_images.py` from the 1600 x 900 set:
the full map background at the screen size, the top bar at `width - 284` x 42, the
right-hand bottom piece at 284 x `height - 600`. With Pillow installed (`python3-pil`) the
script resamples in RGB and quantises back to the original palette; without it, it falls
back to a nearest-neighbour resample of the palette indices.
