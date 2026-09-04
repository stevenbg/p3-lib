# mod-trading-office-details

Fills the trading office window's empty starting page (selected page `-1`, the one
the window opens on) with details about the office, the way
`mod-shipyard-details` and `mod-town-hall-details` fill theirs.

Install: put `trading_office_details.dll` into the `mods` folder (requires the
modloader).

## Page contents

Right-aligned lines, top to bottom:

- The office administrator's **buying discount**: his trade skill makes him pay
  `2 * (50 - level)` percent of every purchase price, i.e. 2% off per skill level up to
  10% at level 5 - a real effect the game never shows anywhere. "No administrator
  employed" when the office has none.
- **Pirates**: how many bands roam (`2 * activity + 1`, fixed at world generation) and the
  home-town rank from which a free pirate robs a merchant (`rank + activity >= 2`), with the
  player's own rank and whether he qualifies.
- **Crop winter**: the months carrying the crop penalty (December to February), whether it
  is in force, and each crop ware's winter output as a percentage, computed from this
  town's own flag word.
- **Dwellings**: the town's houses per class - half-timbered (poor), gabled (wealthy),
  merchants' (rich) - as a count and how full they are, `occupants * 100 / capacity`, the
  figures of the house info panel's "All dwellings in this town" block: the town's per-class
  capacity (`town+0x2FC/+0x2FA/+0x2F8`), the residents of its own houses (`+0x77C/+0x77A/
  +0x778`) plus those of every office's houses (`office+0x2E2/+0x2E0/+0x2DE`), and the house
  count as capacity over the class's per-house capacity (280/140/80). Houses near full are
  the cue to build more; a `*` in front of a class means a house of that type is already on
  the construction list below.
- **Under construction**: every site still being built in this town, grouped into yours,
  other merchants' and the town's, each group in the order the town's building workforce
  pays for them - the building's name, the game's own remaining building time (the number
  the building's info panel prints: `0x0051D6F0`, the sites ahead in the chain each take up
  to 5 of today's builders, then `ceil(work left / min(builders, 5))`, one day less if this
  town's daily pass has already run) and how many builders are on it. A site the day's
  budget never reaches is "waiting": "starts soon" for the first one, "position n" after
  that, the panel's own wording. "Nothing under construction" when the chain is empty.

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
banner graphic, which leaves less room for text.
