# mod-battle-always-flee

The player's ships always try to flee a sea battle the player has not entered.

Install: put `battle_always_flee.dll` into the `mods` folder (requires the modloader).

## Why

A convoy's flagship has to carry guns, and an armed ship is exactly what the battle AI is
willing to fight with. Its fight-or-flee decision (`0x00622804` inside the per-ship step
`0x00621770`) runs like this:

- while the ship is **undamaged** (hull equal to its value at battle start) and **nobody on
  its side has panicked** (the battle's latch at `+0xAC9` is clear), the evaluation is
  skipped and the flee flag cleared: the ship stands and fights;
- once it runs, the tests are squadron ratios - guns against guns, boarding power against
  boarding power, own hull below 25% - and one branch that fires for an **unarmed** ship
  alone: `ship+0x120 == 0` and outnumbered by the other side's boarding power.

So the same auto-trader that fled every pirate while it sailed unarmed will stand and fight
the moment it is given a gun for convoy duty, and it fights until it has taken a hit and
the ratios turn against it. An auto-resolved battle is the same tick-by-tick simulation
as the one the player enters, so this is what decides those fights too.

## What the mod does

Every battle ship is stepped once per tick through slot 0 of its vtable (`0x0067AAD0`, or
the twin `0x0067B0D4`, both `thiscall(this, arg)`). The mod hooks both slots, runs the
game's step, and then, if the ship belongs to the player and is still in the fight, does
what the battle window's flee button does - operation `0x9B`: `+0x12A = (flags & 0x7E) | 1`,
`+0x13E = 0` - and sets the side's panic latch. Because the AI rewrites the flag on every
step, this has to be re-asserted every step; issuing the flee operation once lasts one
step.

A ticker message announces the first forced step of every battle (`Hansa: forced to
flee`). Sunk, captured and disengaged ships are left as they are.

**The battle the player has entered is left alone.** The move order cancels a flee
(`0x00541200` ends in `and [+0x12A],0xFE`), so forcing the flag there would fight the
player's own steering. That battle is the one whose pool slot is in `[0x006E59CC]`, which
the enter/decline operation (`0x96`) toggles against the battle's slot (`battle+0x662`);
every other running battle gets the override.

The two vtable slots are checked against the step addresses before anything is written, so
a different game build refuses to patch and `start()` fails loudly.

## Where the knowledge lives

`p3-api`'s `battle` module: the battle pool, the battle object (side aggregates, the panic
latch, the player-battle slot), the per-ship battle object (fields and `+0x12A` flag bits)
and the battle orders `0x93`..`0x9B` as `Operation` variants.
