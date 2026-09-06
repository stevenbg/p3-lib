# mod-trading-qol

Hotkeys for configuring a trading office administrator, and for editing a ship's trade
route, from live game data, plus a column of checkboxes on the administrator view that
copies chosen orders to your other offices. All prices are computed from the game's price
curves - no configuration files.

Install: put `trading_qol.dll` into the `mods` folder (requires the modloader; `hotkey_registry.dll` from `mod-hotkey-registry` for the hotkeys). Editing
routes also needs `fix_uncompressed_trade_route_loading.dll`, because generated route
files are uncompressed. Keys are dispatched through the shared registry
(`hotkey_registry.dll`, see `mod-hotkey-registry`); without it the mod still loads, with all keys
inert. The office and goods-dialog keys are registered only while their window is on
screen. The global keys pass the keystroke on to the game; the office and goods-dialog
keys consume it, which is what lets F1 mean three different things without ever meaning
two at once.

## Hotkeys

The price keys act only while a trading office window is open, on its administrator view
(the "Trading Office" side button), or while the route window's goods dialog is open. The
others work anywhere in a running game, and take the town whose view is open (on the world
map, the town last visited).

**F1 does three different things depending on where you are**, and only ever one of them
per press. With a trading office window open it is the office setup; with the goods dialog
open it fills the displayed stop; with neither open it builds a 5stop route. The shared
hotkey registry resolves that: the office and dialog register F1 when they open and
unregister when they close, the newest registration wins, and it consumes the keystroke -
so the global route key is simply not reachable while one of those windows is up, and comes
back by itself when the window closes. The goods dialog wins over the office window,
because it opened last.

| Key | Action |
|-----|--------|
| F1 (office window) | Setup: employs an administrator if the office has none (operation `0x5E`, the hire button's own), then every ware without an order becomes BUY if the town produces it, SELL otherwise, at the Q price levels (buy par, sell supply price). Existing orders are untouched; a buy's amount is set to 20 loads / 200 barrels (the same quantity either way) only if it is currently 0. |
| Ctrl + F1 | Provision the celebration goods (beer, wine, fish, meat, grain, honey - the six wares a celebration counts): raise their amounts to what a **top-level celebration for the town's whole population requires in stock** - never lowering - and lock their quantities ("Lock min. store quantity for auto trade ships"), so route ships cannot take the stock. Directions and prices untouched. The amount is the game's own attendance test, a flat per-guest figure per ware (grain 3, meat 2, fish 2, beer 2, honey 1, wine 2) times the guests, rounded up to whole units; both the ware list and the per-guest figures are read from the game's table rather than written out here. Two deliberate choices: guests are the full citizen count, an upper bound since real attendance caps at 99%, and the target is the level requirement rather than the doubled amount a feast actually eats - the second half disappears without improving the celebration |
| Alt + F1 | Provision the building materials, enough for any building or ship (cloth 10, hemp 8, pitch 50, bricks 80, timber 50, iron goods 50): raise the amounts to those - never lowering - and lock the quantities, like Ctrl+F1. |
| Ctrl + Q W E R T Y | Set the BUY prices to that price level (see below): in the route window's goods dialog ("Automatic maritime trading"), every buy order of the stop being edited; otherwise every buy order of the administrator view |
| Alt + Q W E R T Y | The same for SELL prices |
| Q W E R T Y (in a price box) | With a price box selected for typing - click it, in the administrator view or in the goods dialog - the plain key sets **that one ware's** price to the level, the buy price for a buy order and the sell price for a sell. The game commits it exactly as it would typed digits: at once in the office view, and when you leave the row or close the dialog in the goods dialog. A ware without a buy or sell order gets a popup instead. The other keys still type into the box as usual |
| F11 | Dump thresholds, base prices, all price levels and the weekly citizen/business consumptions to the log and to `<TownName>.csv` in the game directory |
| F1 (nothing open) | Rebuild the selected ship's route as a 5stop supply route. The ship's current route provides the towns - its first stop becomes the home town, the remaining unique towns in order the targets (a ship without a route uses the merchant's home town and targets the open town view): load a week of every target's demand at the home office, then per target sell it there and reset that office's stock, and finally haul everything home. Targets without a player office get a combined sell-and-buy stop instead, since office transfers would be wiped there: sells as usual, and buys at the E price for everything except the no-buy wares and what the home town produces itself. The previous route is saved to `_backup.rou` first |
| F2 | The same, but with the 6stop office swap per target |
| F3 | A collection route instead: one buy stop per target at the E price for everything the home town does not produce itself. A single home stop, at the front, transfers the whole hold into the office; the route loops back to it, so there is no home stop at the end |
| F4 | A trade circuit: nothing loaded at home, one self-contained trade stop per target - buy that town's production at the R buy price, **sell everything else** at the R sell price, Max amounts - and everything unloaded at home. "Everything else" includes any no-buy ware the town produces: it is dropped from the buying, not from the stop. No consumption figures and no office needed at any target; the price is the limit rather than a quantity |
| Ctrl + F3 | A fetch route: collect a hand-picked ware list from everywhere it is made. The wares **and their prices** are the buy orders you left on the ship's **first stop**, the targets are every town that produces one of them, and the stops are ordered into the shortest round trip. Every stop buys the whole list at your prices. The single home stop transfers the whole hold into the office before the buying starts, and the route loops back to it - so there is no separate home stop at the end. Needs an existing route to read - see below |
| Ctrl + F1 / F2 | The same templates, but the load is scaled to the route's **actual lap time** instead of a fixed week: the summed leg travel times (the game's own formula, full load and full hull) plus the 6-hour dwell per stop, rounded up to whole days. Combines with Shift and Alt |
| Alt + F1 / F2 | Leaves the low-value industry inputs (bricks, pig iron, pitch, hemp) out of the supplies, freeing hold space for goods with citizen consumption |
| Alt + F3 / F4 | Buys the no-buy wares (pitch, timber, salt, bricks, grain, hemp) as well |
| Alt + Ctrl + F3 | Calls at **every** town rather than only the ones that produce a wanted ware |
| Shift + F1 / F2 / F3 / F4 | Any of the above, but targeting just the currently open town and APPENDING the generated stops to the existing route instead of replacing it (combines with Alt) |
| F1 (goods dialog) | With the route window's goods dialog open: fill the displayed stop's empty ware slots - buy what the stop's town produces at the Ctrl+R price (Ctrl+F1 includes the no-buy wares), sell everything else at the Alt+R price, Max amounts. Existing instructions, office transfers included, stay untouched |
| Alt + F1 (goods dialog) | Set the displayed stop's QUANTITIES to what the route's supplied towns require right now - a week of each one's current citizen and business consumption, all goods, summed over the unique route towns minus the first stop's. Quantities only: prices, directions and the instruction order stay untouched. Meant for refreshing a supply route's home load stop as the towns grow |
| Ctrl + Alt + F1 (goods dialog) | The same, but for the route's **actual lap time** instead of a fixed week: the summed leg travel times (the game's own formula, full load and full hull) plus the 6-hour dwell per stop, rounded up to whole days. A short route loads less than a week, a long one more |
| DEL (office window) | Reset: every ware back to no order, amount 0, and "lock amount" unticked. The inverse of the office F1 - setup only fills in wares that have no order, so this is what makes it repeatable |
| DEL | Clear the selected ship's route entirely (own ships only; refused while the goods dialog is open, since it displays a stop of that route). The route is saved to `_backup.rou` first, and the ship is given a fresh name from the game's own ship-name pool - the route keys name a ship after its towns, so a cleared ship should not keep advertising a route it no longer has |

Alt is a filter everywhere, and on Ctrl+F3 the filter it relaxes is the one over **towns**
rather than wares - the ware list there is written out by hand, so there is nothing left to
widen. Ctrl+F3 takes no Shift: its targets are derived, so Shift's single open town has
nothing to say.

Alt is always a ware filter, never a quantity: it widens what a buying template takes and
narrows what a supplying template carries. On F1 and F2 it does not touch the buy half of
an office-less target's stop - those keep leaving the no-buy wares alone, so one press
cannot free hold space and refill it at the same time.

In the goods dialog the price keys refresh the display the way the dialog's own stop
arrows do, which starts a new Undo session: Undo covers changes made since - the same
as after a stop switch (the game resets its Undo baseline on every stop display).

Directions and amounts are never changed by the price keys; F1 is the only key that
creates orders, and the Set button only copies existing ones.

The route keys take both the home town (its first stop) and the targets (the rest) from
the ship's own route; Shift and F4 use the town whose view is open, with the merchant's
home town as the source. Ctrl+F3 is the exception - it reads the wares off the first stop
and derives the targets from who produces them (see below). Route
quantities are one week of each target
town's real consumption - citizens plus businesses, the market hall consumption
window's Total column, read live from the town - rounded up to whole in-game units;
prices are the Q levels. The t0 threshold is deliberately not used for quantities: it
is a comfortable stock level, inflated by minimum floors and construction reserves, not
what disappears weekly. Wares the town produces itself are not supplied to it; everything
else it consumes is, unless Alt is held, which additionally leaves out bricks, pig iron,
pitch and hemp - low-value industry inputs whose margin rarely pays for the hold space.
Whatever the target office holds of them is hauled home either way. The game
wipes office transfers of stops in towns without a player office at route load time -
the templates avoid generating any such stop (office-less targets get the 3-stop
variant, and the collection route only transfers at home).

Stops load and buy the barrel goods before the bulky loads goods, each group ordered by
ware value with the best first, so when hold space runs out the least valuable cargo is
what gets left behind (see p3-rou's README).

The route keys also rename the ship after its new route: a leading `-`, then the first three
letters of each route town, unique, in route order (e.g. -LueRosSte), up to ten towns
(31 characters, the ship struct's name capacity) - through the game's own rename
operations, so every name display stays in sync. The `-` makes the generated ships
sort together at the top of any name-sorted ship list, and costs no town: one dash
plus ten towns is exactly the 31 characters available. The name is also what the
thawing-port feature below reads to decide which ships serve a town, so renaming a route
ship by hand takes it out of that.

## Copying orders to your other offices: the Set column

The administrator view gets a column of checkboxes left of the ware names, and a **Set**
button in the bottom-left corner with the hint "qty & price across offices". Tick the wares
whose orders you want everywhere, press Set, and for each ticked ware the **amount and the
price** are copied from this office to every other trading office of yours **whose order for
that ware runs the same way** - a buy stays a buy, a sell a sell. An office with no order for
the ware, or one the other way round, is left alone, and so is a ticked ware that has no
order here; the lock checkbox is not copied. The popup reports how many orders were set in
how many offices and how many were skipped.

The checkboxes and the button are the game's own widgets - the same round button and
checkmark as the lock column - and they show only when the view has something to copy: on
the administrator page of an office that employs an administrator. The selection is
forgotten when the window closes, so each office starts blank.

`mod-trading-office-prices-synchronization` does a blanket version of this (every locked
ware, to every office, whenever the window closes); the two can run side by side.

## Right-click on the Options button: the mod's window

A right-click on the Options button - the cogs under the minimap, whose left-click opens
the game menu - opens (and closes again) a window of the mod's own, the place its
configuration will live. Until it has settings to show it lists placeholder rows behind a
game scrollbar. The right-click is not passed on to the game, so it does not also close the
topmost window the way a right-click elsewhere does.

The game routes a right-button release to the pressed and the focused child of the main
scene (`[0x006CBB40]`, the container that owns the side panel), never to the child under
the cursor, so the button itself never sees it. The capture is a hook on the scene's
right-button-up slot (vtable `0x0066C8A0 + 0x148`, `0x004298F0`, called with the cursor in
game coordinates) that tests the release against the button's rectangle (the `CViperButton`
at `scene + 0x16F0`, the one the scene's update polls at `0x00423A73` to open the menu). The
slot is verified to hold `0x004298F0` before it is written. The window is
`p3-api`'s `custom_window` with a `scroll_list` bar (`src/config_window.rs`).

## Crew rescue: stalled routes hire their own sailors

A route ship that loses sailors (pirates, mostly) trades at its next stop and then
stalls: the game deactivates the route and posts `%s's trade route: crew number too
low`. This mod intercepts that message at its single creation site (`0x00519D72`) and,
when the tavern of the town the ship is docked in can cover the shortfall, hires up to
the type's sailing minimum (the game's hire operation, opcode `0x04`) and re-activates
the route (opcode `0x68`, the route window's own button) - the vanilla message never
appears; a ticker line says `<ship>: hired N sailors, route resumed` instead. If the
tavern cannot cover the shortfall, nothing is touched and the vanilla message shows.

## Fetch routes (Ctrl+F3)

The other route keys decide *what* to carry from the towns you picked. A fetch route works
the other way round: you pick the **wares**, and it finds the towns.

The buy orders on the ship's first stop are the specification, prices included, so the
ship's own route says what it is for. Set it up once - open the route window's goods dialog
on the first stop and give each ware you want a buy order at the price you are willing to
pay (Ctrl+Q..Y prices the whole stop off the ladder, or edit a ware by hand) - then press
Ctrl+F3. What you get is:

- the first stop's town as the home town, as with every other route key;
- one buy stop for every town that **produces** at least one of the wanted wares, each
  buying the whole wanted list at **your prices**, copied through ware by ware;
- those stops ordered into the shortest round trip from home and back;
- one home stop, at the front, transferring the whole hold into the home office.

That last point is worth spelling out: the route is a closed loop, so the ship arrives back
at the first stop after the last town and unloads there. A separate home stop at the end
would do the same thing twice, so there is not one - which also leaves a stop spare against
the 20 the auto-trade window can display.

This is the only route key that does not price itself, which is the point: the price is
the whole control surface here. Every other template applies one level to a list it
derived, so it can only offer a single letter for all of it; here you can pay near par for
the bulk goods and reach for something scarce in the same route, and the numbers you see in
the dialog are exactly the ones the route sails with.

Production decides which towns are worth calling at, not what a stop buys once it is
there. A producer keeps refilling between visits, whereas a town that happens to be
holding twenty barrels of wine today is not a source to build a standing circuit around -
but once the ship is in the harbour there is no reason to walk past the rest of the list,
and your price limits are the filter anyway: a stop will not buy into a shortage.

**Alt + Ctrl + F3 drops the production filter** and calls at every town on the map. Worth
it when production is the wrong question: a town can sit on a wanted ware it does not make
- imports, an AI trader's dumping ground, a mill that has since closed - and your price
limits mean the ship buys only where there really is a surplus, so a call that finds
nothing costs sailing time and nothing else. The tour is still the shortest one through all
of them, which is what makes the wider sweep affordable.

The Y price is the most aggressive in the mod, and deliberately so - you named these wares
one at a time, so the route should outbid a producing town's own market rather than sail
past it half empty. It drains a producer down to roughly two weeks of that town's own
consumption (five weeks for grain; see the price model below).

**It is a one-shot expansion.** The generated route replaces the first stop with the usual
empty home bracket, so the buy orders that specified it are gone once it has been built -
pressing Ctrl+F3 again reports that the first stop has no buy orders. The previous route is
in `_backup.rou` as always, and re-marking the wares takes a few clicks. Refusals say which
of the three inputs is missing: no route to read, no buy orders on the first stop, or no
town but home producing any of them.

### Ordering the stops

The tour uses the game's own pathfinder - the one that moves the ships - so a leg is as
long as the water route really is, around coastlines and along the sea lanes, not a
straight line between two dots. It builds the full distance matrix over home plus the
targets, takes a nearest-neighbour tour from home and then improves it by 2-opt until no
segment reversal helps. For the handful of towns a ware list turns up that is optimal or
within a percent of it, and it costs one keypress (all 24 towns would be 276 router calls).

Travel time in this game is that distance divided by a per-ship speed factor - the same
factor on every leg, from the ship's class, hull condition and load - so the shortest tour
is also the fastest one, whatever ship ends up running it. Ordering on distance alone is
not an approximation of travel time; it gives the identical ranking.

The log line names the tour, its length, and what plain nearest neighbour would have cost,
so the ordering is checkable. Only fetch routes are reordered: on the other keys the target
order is the one you put in the ship's route by hand, and rearranging that would be a
surprise.

## Thawing ports restart route ships

Not a hotkey - it runs on the game's own event. A frozen port turns arriving ships away,
and an auto-trade ship routed through one can end up stopped, which is easy to miss and
tedious to restart by hand. When a port thaws, this mod restarts automatic trade on the
route ships that serve that town.

It uses the generated name as the record of which towns a ship serves: the route keys
already write `-` plus one three-letter code per route town, so a ship named `-LueRosSte` is known to
serve Luebeck, Rostock and Stettin without walking its route. On a thaw, every ship of
yours whose name starts with `-` and contains the thawed town's code gets its "active"
checkbox set, through the game's own operation (`0x68`), the same one the checkbox sends.

It only ever *sets* the flag, and only on your own ships: a ship already trading is left
alone, so it cannot stop anything, and it ignores ships you named yourself. Each thaw is
reported on the event ticker ("Stockholm ice-free: restarted 2 ship(s)") and in the log,
which also names the ships. Ports freeze between roughly 3 December and late February,
and only the northern and eastern ones freeze at all - Bergen, Oslo, Stockholm, Visby,
Riga, Reval, Ladoga and Novgorod on the standard map.

This assumes town names are distinct in their first three characters, which holds for all
24 towns of the standard map.

## Price levels

Q to Y ascend in price. Each key is a fixed multiple of the ware's base price, evenly
spaced 0.05 apart:

| key | BUY (ctrl) | SELL (alt) | margin if you buy and sell on the same key |
|-----|------------|------------|--------|
| Q | 1.00 | 1.40 | +40.0% |
| W | 1.05 | 1.45 | +38.1% |
| E | 1.10 | 1.50 | +36.4% |
| R | 1.15 | 1.55 | +34.8% |
| T | 1.20 | 1.60 | +33.3% |
| Y | 1.25 | 1.65 | +32.0% |

A sell limit makes the administrator stop selling once the market fills to the stock where
that price is reached; a buy limit makes it drain the market down to the stock where that
price is reached. So buying gets more aggressive towards Y (it pays more, so it can keep
buying from a town holding less), selling more restrained (it holds out for more, so it
only sells into scarcity).

Every matched pair is profitable, and buying on a key at or below the key you sell on can
never lose money. Q is both the natural pair - buy at par, sell at the price that leaves
the town one week of supply - and the widest margin.

The generated orders use these levels:

| feature | buy | sell |
|-|-|-|
| F1 office setup | Q | Q |
| F1 goods-dialog fill, F4 trade route | R | R |
| F1 / F2 supply routes | E (at office-less targets) | Q |
| F3 collection route | E | - |

Ctrl+F3 is not in that table: it copies the buy prices off the ship's first stop, so its
prices are yours rather than a level of the mod's. Ctrl+Q..Y in the goods dialog is a
convenient way to set them, but nothing forces them onto the ladder.

In stock terms the buy ladder covers the upper half of the t0..t1 segment and the sell
ladder the upper part of 0..t0, so a buy leaves this many weeks of the town's consumption
behind:

| | Q | W | E | R | T | Y |
|-|-|-|-|-|-|-|
| most wares | 3.0 | 2.8 | 2.6 | 2.4 | 2.2 | 2.0 |
| grain | 5.0 | 4.6 | 4.2 | 3.8 | 3.4 | 3.0 |

Grain is the outlier because its t1 is five weeks rather than three (see the price model
below), so every level demands more stock of it before a purchase fires.

## Price model

The game recalculates four per-ware price thresholds t0..t3 every tick
(`TownPtr::get_price_thresholds()`): t0 is one week of the town's consumption - 7 days
x (citizen + business daily consumption + 1), both arrays live on the town struct
(`get_daily_consumptions_citizens`/`_businesses`, verified against the market hall
consumption window) - t1 adds two more weeks (grain: four), t2 adds ten days of the
town's production (verified against `get_production_values()` across four towns - this
is the gitbook's "unidentified array"), and t3 adds another consumption week. Building
materials use fixed bases, tiny values are clamped to minimum steps, and grain/wine get
seasonal t1 bonuses in autumn.

Prices are continuous piecewise-linear curves over the market stock, anchored at the
thresholds, as factors of the ware base price:

- selling (player to town): stock 0 -> the trade difficulty (2.2 low, 2.0 normal,
  1.8 high, read live), t0 -> 1.4, t1 -> 1.0, t2 -> 0.7, from t3 -> 0.5
- buying (player from town): stock 0 -> 4.0, t0 -> 1.5, t1 -> 1.0, t2 -> 0.8,
  from t3 -> 0.6

Because the factors at the thresholds are constants, a price expressed as a fraction
between thresholds is a fixed multiple of the base price - the same number in every
town, and unchanged by a siege. The live thresholds decide which stock that price
corresponds to: when a crisis stretches them, the same price leaves the town
proportionally more goods.

The levels live in `src/prices.rs`, independent of any town or UI, so route creation can
reuse them.

Details: https://p3modding.github.io/towns/ware-prices.html

## Feedback

Every key confirms its action - and explains a refused one (no ship selected, wrong
office view, goods dialog in the way) - as an in-game popup on the scrollmap's event
ticker, the top-left boxes where "Game speed" messages appear. Everything is also
logged with more detail via OutputDebugString.

(The letter-popup town suffix - "Personal letter: Patrol - Stockholm" - lives in
mod-ui-tweaks.)

## Known limitations

- The game prices a transaction as the average of the curve over the amount traded, not
  the marginal price at the end point, so a limit set exactly at a level lets the last
  transaction overshoot that stock point by up to one transaction chunk. Correcting for
  it needs the game's transaction chunk size, which is not reverse engineered yet.
- Log output goes to OutputDebugString (DebugView or a debugger).
