# mod-battle-always-flee

The player's ships flee a sea battle they did not start and the player is not commanding.

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
the moment it is given a gun for convoy duty, and it fights until it has taken a hit and the
ratios turn against it. An auto-resolved battle is the same tick-by-tick simulation as the
one the player enters, so this is what decides those fights too.

## What the mod does

It takes the side's **courage** away for the length of one ship's step, and lets the AI
reach its own decision to flee.

Each of the evaluation's three strength comparisons multiplies the side's own strength by a
courage scale, the byte at `battle+0xAB0` indexed by side - the middle comparison being
`own_gunnery x scale x 4 >= enemy_gunnery`. Battle setup writes `1` to both sides, and `2`
to one side on another path, so the engine varies it itself. At `0`, no comparison can be
met against an armed opponent, so the AI takes its **own** flee branch: it sets the flee
flag, latches the side's panic byte, and steers for the edge of the map.

That is the whole point of going through the AI rather than writing the flee flag directly.
A flag written from outside the step does not survive: the evaluation's fall-through clears
it again (`and eax,0xFE` at `0x00622AF3`) on every step, so the ship keeps fighting. A
decision the AI has made itself is never undone.

Every battle ship is stepped once per tick through slot 0 of its vtable (`0x0067AAD0`, or
the twin `0x0067B0D4`, both `thiscall(this, arg)`). The mod hooks both slots and, for a ship
it should act on, sets that side's scale to `0` immediately before the step and restores the
previous value immediately afterwards. The scale belongs to the side, so bracketing the call
this way is what keeps it a **per-ship** change: an ally sharing the side keeps its real
courage during its own step, and a side that started at `2` goes back to `2`.

A ticker message announces the first affected step of every battle (`Hansa: forced to
flee`).

Nothing else about the fight changes. The scale is read only by the two fight-or-flee
evaluations and by nothing else in the executable - no damage, accuracy, speed or boarding
maths - so a ship made cowardly this way still returns fire, still takes damage, and can
still be caught, boarded or sunk by its pursuer.

### Which battles it leaves alone

Three conditions must all hold, or the mod does nothing:

- the ship belongs to the player and is still in the fight;
- **the player is not commanding the battle.** That battle is the one whose pool slot is in
  `[0x006E59CC]`, which the enter/decline operation (`0x96`) toggles against the battle's
  own slot (`battle+0x662`);
- **the player did not start the battle.** Side 0 is the attacker and side 1 the defender,
  and each side's owning merchant is recorded at setup - `battle+0x35A` for side 0,
  `battle+0x35C` for side 1, read off the side's ship and left at `0xFF` when the side has
  no ship, as a town's does. So a town assault, or any attack the player launches, is
  fought out normally.

What is left is exactly the case the mod is for: something attacks the player and the player
declines to take command.

The two vtable slots are checked against the step addresses before anything is written, so a
different game build refuses to patch and `start()` fails loudly.

## Where the knowledge lives

`p3-api`'s `battle` module: the battle pool, the battle object (side aggregates, the courage
scale, the side owners, the panic latch, the player-battle slot), the per-ship battle object
(fields, the `+0x12A` flag bits and the AI grace counter) and the battle orders `0x93`..`0x9B`
as `Operation` variants.
