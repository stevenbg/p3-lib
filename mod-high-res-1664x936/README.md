# mod-high-res-1664x936

Runs the game at 1664 x 936. Replaces the resolution the game computes from its options
field with 1664 x 936 (three detours on the resolution mapping), repositions the UI
elements laid out by resolution switch, ships replacements for the full map background and
the top bar and side panel pieces at this size, and reads the loading screen's layout from
`accelMap.ini`.

Install: put `high_res_1664x936.dll` into the `mods` folder (requires the modloader).

936 is a multiple of 8, so the town view's light layers need no padding (see
`mod-high-res`'s README for why that matters).

## Images

The three 8-bit BMPs in `src/` are produced by `resize_images.py` from the 1600 x 900 set:
the full map background at the screen size, the top bar at `width - 284` x 42, the
right-hand bottom piece at 284 x `height - 600`. With Pillow installed (`python3-pil`) the
script resamples in RGB and quantises back to the original palette; without it, it falls
back to a nearest-neighbour resample of the palette indices.
