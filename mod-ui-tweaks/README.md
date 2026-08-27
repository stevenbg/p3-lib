# mod-ui-tweaks

Assorted quality-of-life tweaks - the collection point for small features that do
not warrant a mod of their own.

Install: put `ui_tweaks.dll` into the `mods` folder (requires the modloader).

## Extra speed

**Numpad `*`** and **numpad `/`** scale the game's time x1 / x2 / x4 / x8,
anywhere - world map, town view, sea battle. Every change posts a popup on the
event ticker (`extra speed: x4`). Nothing resets the scale behind the player's
back; numpad `/` brings it back down.

They sit next to the game's own speed-slider keys (numpad `+` and `-`) without
colliding with them, and need no modifier: the game's key dispatch
(`0x00424B9B`) handles only numpad `+`, numpad `-`, Pause and Tab, so `*` and `/`
reach nothing but this mod.

The intended use is sea battles, which run at a fixed pace the speed slider cannot
touch. The keys are deliberately global rather than battle-scoped: battles are not
a map type of their own - a town attacked from the sea fights on the town's own
map - so there is no reliable "in battle" test on the loaded map, and a global
speed control is simply more useful.

### Why time itself, and not the game speed

The world's speed system never reaches the local-map simulation:

- World time only advances through operations: a tick pacer (`0x00546640`) converts
  elapsed real milliseconds into advance-time operations (opcode `0xC4`), sized by a
  mode field (`operations+0x92C`): 0 = normal play, where the 6-position speed
  slider sets the ms-per-tick divisor (`+0x8D4`, 3515 down to 78); 1 = the
  fast-forward mode with its own window (`+0x8D8`, 2 ms/tick); 2 = local map,
  a fixed 3375 ms/tick from the constant `[0x673CF8]`, slider ignored. The speed
  buttons enqueue opcode `0xC8` (`Operation::SetGameSpeed` in p3-api).
- The local-map simulation (ships, projectiles, battle AI) ignores all of it, and
  ignores how often it runs too: its per-frame update (`[0x6E51AC]`'s vtable
  `+0xF4` = `0x0058B7F0`) measures elapsed real time itself - calling it eight
  times per frame moves nothing, verified in game. The only clock it believes is
  the frame clock.

So the mod dilates the frame clock. The updater at `0x004BD180` (once per frame,
the game's 50 fps limiter inside) computes this frame's real elapsed ms and both
stores it to the frame-delta global (`[0x6DCCF4]`) and accumulates it into the game
clock (`[0x6DCCF8]`). The 5-byte delta computation at `0x004BD1E2` is detoured to
multiply by the scale first, so everything paced by either global sees faster
time - the battle and the world alike, which matches vanilla, where the world keeps
running during battles anyway.

The keys are polled once per frame from the clock updater's two call sites
(`0x004B8AE2`, `0x004B8F7E`), so the mod installs no OS keyboard hook and cannot
collide with other mods' keys.
