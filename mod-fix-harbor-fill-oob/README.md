# mod-fix-harbor-fill-oob

Fixes a vanilla crash-to-desktop (and, more often, silent heap corruption) when
opening a town view, caused by the harbor flood fill running off the map edge.
Observed once in the wild: clicking Ripen on the world map at the moment a convoy
was leaving its port.

Install: put `fix_harbor_fill_oob.dll` into the `mods` folder (requires the
modloader).

## The bug

When a town view opens, the scene rebuilds a per-map harbor-region object
(`0x0062C730`). The rebuild picks seed waypoints from the map's list -
the one nearest an anchor point, plus every waypoint within ~35 tiles of it -
pre-steps each seed one tile diagonally with the coordinate stepper `0x0058B740`,
and flood-fills the region matrix from the stepped positions, marking cells
`0xFFFF`.

Nothing in that chain checks bounds. The stepper never clamps, and neither the
seed fill (`0x0062BD50`) nor the recursive neighbour fill (`0x0062C0A0`) validates
its coordinates before indexing the matrix with `stride * y + x`. A seed within
one row of the map edge therefore steps to `y = -1` and indexes the matrix at a
negative offset:

- the fill first **reads** the out-of-bounds cell - a crash when the heap happens
  to place the matrix right after a no-access page (that is the captured crash:
  read at matrix `- 0x36`);
- when the memory below is readable, the very next instruction **writes `0xFFFF`
  there** (`0x0062BD8A`) - two bytes of silent heap corruption per off-map seed,
  and the fill keeps walking.

Whether an edge waypoint gets seeded depends on dynamic state (which waypoint the
anchor picks - a departing convoy near the map-edge sea exit is the observed
suspect), and whether a bad seed crashes rather than corrupts depends on heap
layout. Hence: rare, unreproducible crashes, and corruption that surfaces as
unrelated weirdness later.

## The fix

Both fill entries are detoured to a guard that rejects any coordinate outside
`[0, stride) x [0, height)` (the scene's `+0xC314`/`+0xC318`). A rejected call
returns immediately - byte-for-byte what an in-range fill does when the cell does
not match, so in-range behaviour is unchanged. Guarding `0x0062C0A0` covers the
whole recursion, since it calls itself for matching neighbours.

Every rejection is logged to DebugView (`warn!`) **and appended to
`_harbor_fill_oob.log` in the game folder** with the coordinates, the map's
dimensions and its id. Each line in that file is a crash or corruption that did
not happen. The file logging is temporary evidence-gathering while the fix soaks
and will be removed later; the DebugView warning stays.
