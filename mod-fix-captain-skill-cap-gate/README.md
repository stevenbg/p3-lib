# mod-fix-captain-skill-cap-gate

Lets the player's captains train trade and combat up to their real ceilings, and stops
new auto-traders being rolled above those ceilings in the first place.

## The bugs

**1. The navigation cap gates all three skills.** The ten-day captain scan
(`0x004DCEA0`) decides whether to grow a captain by comparing all three of his skills
against a single threshold `T` - and `T` is the *navigation* ceiling of his slot,
`SKILL_CAPS[index & 3]`, read from the gate table at `0x00672824`. Trade and combat are
never measured against their own ceilings, so:

- they freeze in `[T, T+49]` even when their real caps are 100 points higher (a gain is
  `rand % 0x33`, so one roll from `T-1` can land 49 points past `T`, and then no roll for
  that skill is ever enqueued again);
- once all three sit at or above `T`, the ship is skipped outright and that captain is
  finished for good.

The AI branch has no gate at all, so this holds back **only the player's captains**, and
it bites on half of them: `T` is 250 for two slots in four, 200 for one, 150 for one.

**2. Records are born above their own caps.** The initializer `0x004FDF50` rolls each
skill roughly uniformly over 0..255 under a *total* budget of 600 points
(`0x004FE027`), and never consults the per-slot caps. A fresh campaign2 start has 46 of
~68 live records over cap. Bug 1 hides this today: a frozen record is skipped, so
nothing ever clamps it.

The two are entangled. The gain handler writes a skill's cap **unconditionally** when
`current + gain > cap` - even when that skill's gain is zero - so fixing bug 1 alone
would expose every over-cap record to pruning and *cost* the player skill, which is the
opposite of the point.

## The fix

**Patch 1 - four bytes at `0x00672824`:** raise the gate table to `250 250 250 250`.

`T` stops binding, while the real ceilings keep being enforced by the gain handler's own
table at `0x00673B34`, which this mod does not touch. A refused roll and a
clamped-to-cap gain have the same outcome, so this is behaviourally equivalent to a
correct per-skill gate:

| Case | correct per-skill gate | `T` = 250 |
|-|-|-|
| skill below its cap | enqueue, gain clamped to cap | same |
| skill at its cap | refused, nothing happens | enqueued, handler writes the cap = no change |
| all three at their caps | ship skipped | operations enqueued, none of them change anything |

Growth *rate* is identical, because vanilla rolls a single skill and gives up if that
skill is at its cap either way. The only cost is a queue slot burnt where vanilla
skipped the ship, and that is bounded: the scan considers only ships whose
`captain_index & 7 == group`, about an eighth of a fleet per run, against a budget of
`0x34 - pending`. A forty-ship fleet spends ~5 extra slots per run; it would take ~416
captained ships to exhaust the budget, and the AI never competes for it (its branch
dispatches directly at `0x004DD0C2` instead of queueing).

The gate table is read from exactly two sites, both in the human-owner branch
(`0x004DD0EC`, `0x004DD0F7`), so this **cannot** touch AI captains.

**Patch 2 - clamp at creation:** hook the initializer's single call site
(`0x00509897`), which hands over both the record and its array index, and clamp the
three rolled skills to `skill_caps(index)`.

Captains and pirates alike, because the caps are the same table the game already
enforces on both - this only moves enforcement from "first gain event" to "at birth".
The freed points are deliberately **not** redistributed into the other skills: that
would make some records stronger than vanilla, while a plain clamp only removes what the
caps said should not exist. The captain/pirate flag is still read, for the wage - specifically,
for whether the wage the record carries was computed from the skills just clamped. A
pirate's was, inline in the initializer (`0x004FE104`) and therefore before this hook
ran, so it is recomputed (`0x004FE190`). A captain's was not: the initializer writes `0`,
and the callers that want a real wage compute it after the allocator returns - the tavern
path at `0x00526A87`, which skips it for pirates exactly because the initializer already
did it. So the wage a captain demands in the tavern is derived from his clamped skills
and stays consistent with them.

There is exactly one allocator (`0x005097C0`), so one hook covers every record the game
rolls - town initialization (one captain and one pirate per town), the periodic tavern
replenisher, pirate ships, and administrators. A **new free-play game therefore starts
with zero over-cap records.**

## What changes in an existing save

Records already over cap were rolled before this mod existed, and the game's own clamp
prunes them the first time a gain reaches them - navigation 240 in a 150-cap slot
becomes 150, a visible drop from displayed level 5 to 3. This is deliberate: it is
already what vanilla does to any player captain who is not fully frozen (measured: a
captain sat at navigation 177 for eight years in taverns, was hired, and read 150 nine
months later). The fix makes that uniform instead of arbitrary, and patch 2 removes its
cause going forward.

Pirates are outside bug 1 entirely - handing a ship to a tavern pirate sets the ship's
owner to `0xFF`, and the scan walks merchant ship chains, so an ownerless ship is in
nobody's chain. Their growth is the ungated hideout `+50/+50` award at `0x00514C93`,
which applies the same caps.

## Logging (temporary)

`_captain_skill_gate.log` in the game folder, appended and never truncated, plus `warn!`
to DebugView. It exists to answer "is the fix working" and comes out once that is
settled, along with the observation hook:

- **every creation clamp**: which skills were cut, from what to what, that slot's caps.
- **every skill change on a human-owned captain**: before -> after with the caps and the
  rolled gains, so gains past the old freeze point and one-time prunes of legacy records
  are both visible.
- **queue pressure**: the pending-operation count with each gain, and a peak in the
  periodic summary, to replace the arithmetic above with measurement.

The third hook, on the gain handler's call site (`0x00535973`), is **observation only** -
it calls through and changes nothing, and is kept separate from the two patches so it can
be deleted without touching them.

## Verification

F9 in `mod-crash-reporter` is the captain census: it prints every
record as `trader 67 CAPT nav 172/250 trade 86/200 combat 129/150 ... GATED OUT`, marks
over-cap skills, and appends to `_probe1.log` so presses years apart diff directly.

1. `GATED OUT` on the player's captains falls to near zero and stays there - today it
   only grows.
2. A player captain whose navigation cap is below his trade or combat cap keeps gaining
   past it. Nothing in the game does this today.
3. A fresh 1300 free-play game shows no over-cap record anywhere, captain or pirate.
4. AI rows unchanged in character: a flat +8 on all three per eligible round.

Pick the save carefully: the scan returns immediately once its round counter passes
`0x1F`, and a fresh campaign2 reads 117 - dead for its first nine months. Use the
open-ended 1300 game or campaign1. Runs are 10 days apart and each captain group is
eligible about four times a year, so a visible result needs a year or two of game time
between presses.

Background: `.claude/notes/todo/captain-navigation-cap-bug.md` and
`.claude/notes/done/captain-experience.md`.
