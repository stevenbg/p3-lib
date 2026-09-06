# mod-fix-church-anim-crash

Fixes the game crashing when the church window opens after a save load
(access violation at `0x0046B49D`, a write through a NULL pointer).

## What ships now: the root fix

**Updated 29 Aug 2026.** The crash's precondition is a single vanilla defect: the church
window's mode field (`+0x1D30`) is **never initialised by its constructor**
(`0x005C88E0`), which initialises the neighbouring `+0x1D5F` two instructions before
returning and misses this one. Only four sites in the executable write the field - the
close method (to `-1`), `set_mode`, and two page setters - so a freshly constructed window
carries whatever its heap block came with.

Two faces, both seen in play:

- **The crash.** After a quit-to-menu the window is rebuilt from a *recycled* block holding
  a stale mode `> 0`, and the tick streams a frame into a NULL array.
- **A blank first page.** On a cold start the block reads `0`, so the window dispatches page
  0 rather than the `-1` empty page it settles on after its first close. Harmless in
  vanilla - but it is the same field, and it is what made the defect visible:
  `mod-church-details` draws on page `-1`, and its page was missing on the first church of
  a session and present on every one after.

So the mod now hooks the constructor's single call site (`0x00426BBD`) and writes `-1` -
the same value the close method uses - into the mode of the object it returns. That removes
the precondition instead of catching the consequence, and fixes the cold-open page as a
side effect.

**The previous fix is kept, dormant.** Three byte patches in the tick plus guards on the
animation player's unguarded methods live on in `src/tick_patch.rs`, compiled but never
called. If the crash ever returns, call `tick_patch::install()` from `start()`: the two are
independent and can run together. Everything below describes that fix.

## The bug

Building-interior windows share one animation player (a global object recreated
on every save load). Its frame-slot array is only allocated by
`switch_animation` (`0x0046B710`); `load_next_frame` (`0x0046B110`) stores the
frame it just loaded straight into the array. The church window's tick decides
between the two calls purely by comparing animation ids: *"the current
animation is not my one-shot intro, so it must already be my loop animation -
just stream its next frame."*

Two window states break that assumption:

- **After a save load.** The church window is recreated too, but its
  constructor never initializes the mode field (`+0x1D30`), and the new window
  usually inherits the previous session's mode through the recycled heap block.
  A stale mode > 0 makes the tick animate immediately - against a freshly
  constructed player whose current-animation id is the `0xFF` sentinel and
  whose frame array is NULL. `load_next_frame` writes through NULL: crash.
- **Another building's animation is current.** The same branch then streams
  church frames into an array sized and cursored for the other animation - a
  silent heap overflow instead of a crash.

The window's own set_mode epilogue (`0x005CA166`) handles the identical
situation correctly (switch unless the right animation is already playing);
only the tick got the comparison wrong.

## The fix

Three bytes in the tick, changing "not the intro" into "exactly my loop
animation" and routing every other state through `switch_animation`, which
allocates the array, loads frame 0 and positions the sprite:

| Address      | Original           | Patched            | Effect |
|--------------|--------------------|--------------------|--------|
| `0x005C95E2` | `jne` rel8 `0x59`  | rel8 `0x47`        | intro-branch mismatch falls into the loop dispatcher below instead of calling `load_next_frame` raw |
| `0x005C9630` | `cmp eax,esi`      | `cmp eax,ebp`      | compare the current animation against the loop id, not the intro id |
| `0x005C9631` | `jne` (`0x75`)     | `je` (`0x74`)      | equal streams the next frame; anything else switches properly |

All six mode/flag/current-animation combinations were enumerated: every path
that worked before behaves identically, and the two broken ones now play the
loop animation instead of crashing or corrupting.

## Logging (temporary)

To show the fix engaging, the tick's `switch_animation` call (`0x005C9635`) is
hooked: any invocation whose prior state would have crashed (`NULL-array`) or
corrupted (`wrong-array`) is appended to `_church_anim_fix.log` in the game
folder, with the player and window state. The one transition vanilla also
handled (intro finished, hand over to the loop) passes silently.

As belt and braces, the player's three unguarded methods (`load_next_frame`,
`tick` `0x0046B530`, `draw` `0x0046B6A0`) are detoured to bail out and log when
the frame array is NULL. With the tick patch in place these should never fire
at all: the only other user of the player is the tavern window, and an audit of
all of its call sites shows the correct pattern throughout - every stream call
is guarded by a current-animation check, mismatches go through
`switch_animation`, and its teardown call even resets the current-animation id
to `0xFF` by hand (`0x005CD503`). The guards are pure insurance; a rejection
line would mean the audit missed a path. Each site mutes after 25 lines per
session; the guard keeps rejecting.

The file logging comes out once the fix has soaked.
