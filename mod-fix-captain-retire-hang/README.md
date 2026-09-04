# mod-fix-captain-retire-hang

Fixes a **hard freeze** when a captain retires: the game locks up with one core at 100%,
no error and no crash report, and the session is lost.

Install: put `fix_captain_retire_hang.dll` into the `mods` folder (requires the modloader).

## The bug

When a captain passes 50 years the ten-day scan schedules a task to take him off his
ship, recording which ship he was on. Because he might move before the task runs, the
task's first job is to re-find him - and that search contains an infinite loop. Two
instructions compare the wanted captain against a ship's captain and jump back to the
comparison when they differ, without ever advancing to the next ship, so a mismatch spins
forever.

Two more mistakes in the same handful of instructions show the code was never exercised:
the step that should advance to the next ship in the owner's chain is missing entirely,
and the register holding the merchant loop's counter is overwritten with something else.

For the freeze to happen the captain has to leave the recorded ship in the short window
before the task runs - dismissed, reassigned, or the ship sunk or sold. That window is one
tick for your own captains, but **three hours of game time for an AI merchant's**, and AI
ships are sunk and sold constantly, so most of the exposure is other merchants' fleets
rather than your own.

Reproduced deliberately (August 2026) by scheduling the task with a mismatched pair: the
game froze immediately, burning exactly one core, with nothing written to any log because
nothing crashes.

## The fix

The mod resolves the ship before the game's broken search can run:

- **Normally nothing happens.** If the recorded ship still carries the captain - every
  ordinary retirement - the game's own handler runs completely untouched.
- **If the captain moved**, the mod finds the ship he is actually on and points the task
  at it, so he retires from there. This is what the broken code was trying to do.
- **If he is on no ship at all**, the task is dropped - the same thing the game does when
  its own search comes up empty, and the right answer, since there is no ship to take him
  off. The mod also clears his retirement flag in that case, because the game sets a flag
  when it queues a removal and then skips that captain forever while it is set: without
  clearing it, a captain who was dropped here would never retire at all once somebody hired
  him again.

Nothing else changes: what retirement *does* is still entirely the game's own code, which
varies by where the ship is. The mod only decides which ship that code works on.

A rejected alternative: the handler can postpone itself by three hours, and it does so
when the ship is leading a convoy. Reusing that for a moved captain would mean a captain
who left his ship for good gets rescheduled every three hours forever - no freeze, but
endless churn.

## Feedback

Silent in normal play. When it does intervene it writes one line to
`_captain_retire_fix.log` in the game folder naming the captain, the ship recorded, and
the ship he was found on - or that he was on none. Also logged via OutputDebugString.
