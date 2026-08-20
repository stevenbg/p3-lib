# mod-trading-office-details

Fills the trading office window's empty starting page (selected page `-1`, the one
the window opens on) with details about the office, the way
`mod-shipyard-details` and `mod-town-hall-details` fill theirs.

Install: put `trading_office_details.dll` into the `mods` folder (requires the
modloader).

## Page contents

- the office administrator's **buying discount**: his trade skill makes him pay
  `2 * (50 - level)` percent of every purchase price, i.e. 2% off per skill level up
  to 10% at level 5 - a real effect the game never shows anywhere
- **captains for hire**, by town: their navigation, trade and combat levels and
  their daily wage. A captain is listed while his record is chained to a town (that
  is where he sits) and no merchant employs him
- **pirate captains**, by town: their levels and the share of the loot they demand
  (`25 + 5 * ceil(field_8 / 32)` percent). Every town keeps one pirate record
  permanently, so this lists the pirate belonging to each town rather than the ones
  currently sitting in a tavern; pirates out sailing for a merchant drop off the
  list because their record leaves the town's chain

Both lists cover the towns the player has a trading office in. Hold **F1** to list
every town instead.

## How it works

Windows of this class family have a draw method at vtable `+0x9C`, a per-frame
update method at `+0xF4` and open/close at `+0x120`/`+0x118`. Both the draw and the
update method load the selected page into `eax` with a 6-byte
`mov eax, [esi+0xecc4]` and dispatch through a jump table, so both can be detoured
by replacing that load with a jump and returning the same value in `eax`.

This mod hooks all three phases, matching what `mod-shipyard-details` and
`mod-town-hall-details` do:

|Phase|Where|What for|
|-|-|-|
|open (`+0x120`)|vtable hook|set the class48 drawing state once, so the page's text is not clipped|
|update (`+0xF4`, `0x005D9500`)|detour at `0x005D9508`, continue `0x005D950E`|register the window's area as changed (`0x004B9650`) while the page is shown|
|draw (`+0x9C`, `0x005D95A0`)|detour at `0x005D9674`, continue `0x005D967A`|render the page text|

Submitting the changed area matters: without it the page keeps showing older pixels
until something else submits the region (a mouse move over it, or alt-tabbing back
into the game). It has to happen in the **update** phase - the town hall window does
it there too, gated by its day timestamp at `+0x1930`, which is why
`mod-town-hall-details` zeroes that timestamp from its draw hook. Calling it from
inside the draw method instead - whether once or on every frame - leaves the
background art torn and the text flickering.

The page draws no window title: the game's own page `-1` has none, and
`render_window_title` (`0x00420C70`) would spend the top of the window on the title
banner graphic, which the tables need for rows.
