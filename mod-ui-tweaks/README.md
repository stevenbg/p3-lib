# mod-ui-tweaks

Assorted quality-of-life tweaks - the collection point for small features that do
not warrant a mod of their own.

Install: put `ui_tweaks.dll` into the `mods` folder (requires the modloader; `hotkey_registry.dll` from `mod-hotkey-registry` for the hotkeys).

## Extra speed

**Numpad `*`** and **numpad `/`** scale the game's time x1 / x2 / x4 / x8,
anywhere - world map, town view, sea battle. Every change posts a popup on the
event ticker (`extra speed: x4`). Nothing resets the scale behind the player's
back; numpad `/` brings it back down.

They sit next to the game's own speed-slider keys (numpad `+` and `-`) without
colliding with them, and need no modifier: the dispatcher that owns the
speed-slider keys (`0x00424B9B`) handles only numpad `+`, numpad `-`, Pause and
Tab. (It is one dispatcher among several - windows and scenes take their input
through their own methods - but no handler for `*` or `/` shows up in a full-exe
scan, and none has surfaced in play.)

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

The keys are dispatched through the shared hotkey registry (`hotkey_registry.dll`, see
`mod-hotkey-registry`), registered session-globally; without the registry they are inert
while the time-scale detour itself keeps working (at whatever scale was last set,
i.e. x1 on a fresh start).

## CTRL+S / ALT+S / CTRL+ALT+S manage the crew in port

With a ship selected and docked in a harbour:

- **CTRL+S fills to the maximum** the game itself would hire: the type's full crew (the
  byte table at `0x673664`: 10/16/30/24) plus a cargo-derived term, bounded by the room
  left - the hire cap the game computes at `0x005184F0`, the same routine the tavern's
  Sailors page clamps its input with.
- **ALT+S hires the bare sailing minimum**: up to the type's minimum (Snaikka 5, Crayer
  8, Cog 10, Holk 12 - read from the game's own table at `0x673660`, not hardcoded).
  The cheap option for a trader that only needs to move.
- **CTRL+ALT+S dismisses the whole crew** into the town: the game's dismiss operation
  (opcode `0x05`, handler `0x00537DD0`) returns them as beggars and citizens and zeroes
  crew and morale.

A ticker message says what happened (`hiring 4 sailors toward the minimum (1 -> 5)`), or
why nothing did (at sea, already sailable/full, no sailors aboard, tavern empty, tavern
ran short). The hires go through the real hire operation (opcode `0x04`, handler
`0x00537C20`), which re-clamps every request and draws down the merchant's sailor pool
(`0x004F6CA0`) and the town's beggars. The keys are consumed only when a player ship is
selected; otherwise the game still sees them.

## CTRL+X strips a ship bare

Everything a captured prize needs, in one press. With a player ship selected and docked
in a town where the player has a trading office:

1. **all cargo into the office**, ware by ware (opcode `0x08`, handler `0x00538100`,
   which credits the office at the price the ship carried the goods at);
2. **all artillery and cutlasses into the office** (opcode `0x09`, handler
   `0x00538210`): one operation per gun type present, counted off the ship's 24 slot
   bytes at `ship+0x13C`, plus the cutlasses from `ship+0x154`;
3. **the whole crew dismissed** into the town (opcode `0x05`), as CTRL+ALT+S does;
4. **a repair ordered** at the town's shipyard (opcode `0x03`, handler `0x0052ACD0`).

The repair goes last because it puts the ship into status `4`, and every transfer above
requires a status below `4`. **It also charges the full repair price immediately**, with
no confirmation - that is the point of the key, but it does spend money.

Refused for a **convoy's lead ship**: the weapons operation rejects one anyway unless the
convoy carries flag `0x2`, so the key declines the whole sequence rather than half-doing
it. Also refused when the ship is not docked, or when there is no office in the town to
put anything into. A ticker message reports what moved (`Hansa stripped in Lübeck:
9 wares, 4 guns, 30 cutlasses, 18 sailors - repair ordered`).

The operations are executed rather than queued: a fully loaded ship is more than thirty
of them, which is most of the pending queue's 52 slots.

## CTRL+R books a repair

With a player ship selected and **lying in port** (status `0` - a ship still entering the
harbour or already casting off is not a candidate, which is also what the shipyard window
itself lists), the key orders the repair the shipyard's button orders: operation `0x03`,
whose handler `0x0052ACD0` charges the price immediately, books it to the merchant and puts
the ship into status `4`, from where the next ships tick moves it into the yard's repair
chain.

**A convoy repairs as a whole.** The handler sums the cost over every member of the convoy
the ordered ship belongs to, so the order is placed on the convoy's lead ship
(`convoy+0x10`), exactly as the game's own convoy route stop does (`0x00503266`,
`0x0050336B`) - whichever member is selected.

Nothing is pre-checked beyond ownership and being docked: an undamaged hull or a short
purse is answered by the game's own letter. A ticker message reports the order and the hull
as the shipyard shows it (`Hansa: repair ordered in Lübeck - hull at 62%`, or `... for the
whole convoy - lead ship Anna at 62%`), or why nothing happened (`not docked - at sea`).
The key is consumed only when a player ship is selected; otherwise the game still sees it.

## PAGE UP / PAGE DOWN walk the towns you can enter, HOME goes home

Inside a town, **PAGE DOWN** takes you to the next town you may enter and **PAGE UP** to
the previous one, in the game's town order and wrapping around. **HOME** enters your home
town, from inside a town or from the world map. Building windows open in the town are
closed first, the way any opening window closes the others (`0x00462280`).

From the world map the scene has to be swapped as well, and the mod does it the way the
game enters the home town at the start of a session (`0x004339A1`..`0x00433A52`): the
loader is told to reload (the map kept in the hidden scene is not the one on screen), the
world map is centred on the town for the way back (`0x0044F490` with the town's position
from the table at `0x006DDBA0`), the scrollmap is hidden and disabled, the local map shown
and enabled (`+0xCC`, `+0x34`).

"May enter" is the scrollmap's own double-click test (`0x0044A128`..`0x0044A1A6`): one of
your ships names the town (`ship+0x39`) and is not in raider status `0x11`, or you have an
office there. The switch is the loader the scrollmap and the ship overview use,
`0x0058A4F0` on the local-map scene, called with the scrollmap's arguments
`(town, 0, 1, 1, 1)`; a different town releases the loaded map and loads the new one, so it
works from inside a town as well as from the world map. A ticker message names the move
(`Lübeck -> Rostock (3 of 7 towns)`).

PAGE UP and PAGE DOWN are consumed only while the town view is **on screen**, and none of
the three keys while you are **in a sea battle** - the same refusal the game's own
home-town entry makes. Neither follows from the loaded map id: it keeps naming the last
town after you have left for the world map (only the scene is hidden, `+0xCC(0)`), and a
town attacked from the sea fights on the town's own map. So the tests are the scene's
visibility and the battle pool's player slot (`[0x006E59CC]`, `0xFF` when you are in no
fight). In battle the game still sees the keys, and PAGE UP / PAGE DOWN also on the world
map.

## Pirate attacks no longer interrupt fast forward

When a notorious pirate robs someone else's ship the game announces
`<pirate name> has struck again` - a video, or a ticker message if event videos are off -
and **drops you back to normal speed**. It happens often enough to make fast forward
unusable.

While the game is **in fast forward**, this mod suppresses that one event entirely: no
video, no message, no speed change. At every other speed it is left completely alone, so
nothing is hidden while you are actually watching. No other event is affected - sieges,
blockades, plagues, fires and ships finished all still interrupt.

### How it works

The announcement is two calls, not one:

```
60eb53:  je   0x60ebb6        ; the victim is you -> vanilla skips the announcement here
60eb55:  mov  ecx,[0x006CC7E8]   <-- the only way in
60eb5d:  call 0x00469380         ; open and SHOW the window
60eba7:  call 0x00469AF0         ; populate it
60ebac:  mov  eax,[0x006DE4B4]
60ebb1:  mov  [0x006E59D8],eax   ; stamp when the last event fired
60ebb6:  ...                     <-- the only way out
```

**Both calls have to go together.** Suppressing only the populate leaves the window on
screen with nothing in it, and the render loop faults indexing `window+0x3BC` with the
still-`-1` event type. That was a real crash during development, not a hypothetical.

So the mod detours the region's single entry instead of hooking either call: in fast
forward it jumps straight to the single exit, and otherwise performs the instruction it
replaced and carries on. Nothing branches into the region's interior - checked across the
whole executable - and the skip reuses a path the game already has, since the `je` one
instruction earlier takes the same exit when the robbed merchant is you.

It also skips the `0x006E59D8` stamp at `0x0060EBB1`. That is a **30-day cooldown**: its
only reader (`0x0060E9B3`) tests `stamp + 30 days < now` to re-arm a fallback that lets an
announcement through when it would otherwise be skipped. Leaving it alone means a
suppressed event does not consume the quota, so announcements are throttled per 30 days of
*visible* play rather than per 30 days of game time - you may see them a little more often
at normal speed than vanilla would. The alternative is worse: stamping while suppressing
would let an announcement you never saw silence a later one you would have.

The six replaced bytes are verified before anything is written, so a different game build
refuses to patch rather than corrupting code.

## The auction is announced late, not a day early

Vanilla announces `Tomorrow there will be an auction in %s.` at **midnight**, and
holds the auction at midnight the following day - a full 24 hours later. That
announcement also stops fast forward, so it is easy to be interrupted, forget, and
still miss the auction. This mod moves the announcement to **22:30 the same day**,
1.5 hours of warning. **The auction itself does not move.**

### How it works

The auction is a scheduled task (kind `0x0A`, handler `0x004E2CD4`). Task due times
and the game clock (`[0x006DE4B4]`) are both in **1/256 of a day**, so the low byte
of a due time *is* the time of day and `mov byte [esi],0` means "align to midnight".
Three of those exist in the auction task; the mod rewrites the immediate of two:

|Address|What it schedules|Patched|
|-|-|-|
|`0x004E2DFC`|the announcement|yes|
|`0x004E2EFB`|the announcement again, when the town already has an auction running and this one slips a day|yes|
|`0x004E2FDA`|**the auction itself**|no - moving it would drag the auction along and change nothing about the gap|

The announcement state then adds a whole day and re-aligns to zero, discarding the
offset, so the auction stays at midnight whatever the announcement is set to.

Retuning is one constant, `AUCTION_ANNOUNCE_TIME_OF_DAY`: `0x80` = noon (12 h of
warning), `0xC0` = 18:00 (6 h), `0xE0` = 21:00 (3 h), `0xF0` = 22:30 (1.5 h, the
default), `0x00` = vanilla. `0xFF` is one tick and not useful.

Both sites are verified to read `c6 06 00` before anything is written, so a
different game build refuses to patch and `start()` fails loudly rather than
corrupting code.

## Letter popups name their town

The incoming-letter notifications on the top right read "Personal letter: Patrol -
Stockholm" instead of "Personal letter: Patrol", so the town a mission wants the ship in
is known without opening the letter.

Simple letters carry a usable town byte. Scripted letters do not: their town byte is
the low byte of whatever script variable the letter was created with, which need not be
the town the letter is about - and for a patrol letter it is garbage unless the
corrected `patrouille.p2m` from `mod-fix-patrol-letter-crash` is installed. For those
the town is recovered from the formatted text instead: the last town name occurring in
it, which is the one the letter tells the player to sail to.

Two call hooks do it (`src/letter_popups.rs`): the mailbox insert's announcer call
(`0x004D66E0`) stashes the message being announced, and the announcer's right-ticker
enqueue call (`0x004D7D12`) rebuilds the popup string with the town appended. The
enqueue takes its string by value and releases it, so the hook releases the incoming
one and hands the original a fresh one.
