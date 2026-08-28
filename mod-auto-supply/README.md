# mod-auto-supply

Hotkeys for configuring a trading office administrator, and for editing a ship's trade
route, from live game data. All prices are computed from the game's price curves - no
configuration files.

Install: put `auto_supply.dll` into the `mods` folder (requires the modloader). Editing
routes also needs `fix_uncompressed_trade_route_loading.dll`, because generated route
files are uncompressed. Keys are dispatched through the shared registry
(`hotkeys.dll`, see `mod-hotkeys`); without it the mod still loads, with all keys
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
| F1 (office window) | Setup: every ware without an order becomes BUY if the town produces it, SELL otherwise, at the Q price levels (buy par, sell supply price). Existing orders are untouched; a buy's amount is set to 20 loads / 200 barrels (the same quantity either way) only if it is currently 0. |
| Ctrl + F1 | Provision the celebration goods (beer, wine, fish, meat, grain, honey - a celebration needs them in stock): raise their amounts to a week of the town's citizen consumption - never lowering - and lock their quantities ("Lock min. store quantity for auto trade ships"), so route ships cannot take the stock. Directions and prices untouched. |
| Alt + F1 | Provision the building materials, enough for any building or ship (cloth 10, hemp 8, pitch 50, bricks 80, timber 50, iron goods 50): raise the amounts to those - never lowering - and lock the quantities, like Ctrl+F1. |
| Ctrl + Q W E R T Y | Set the BUY prices to that price level (see below): in the route window's goods dialog ("Automatic maritime trading"), every buy order of the stop being edited; otherwise every buy order of the administrator view |
| Alt + Q W E R T Y | The same for SELL prices |
| F11 | Dump thresholds, base prices, all price levels and the weekly citizen/business consumptions to the log and to `<TownName>.csv` in the game directory |
| F1 (nothing open) | Rebuild the selected ship's route as a 5stop supply route. The ship's current route provides the towns - its first stop becomes the home town, the remaining unique towns in order the targets (a ship without a route uses the merchant's home town and targets the open town view): load a week of every target's demand at the home office, then per target sell it there and reset that office's stock, and finally haul everything home. Targets without a player office get a combined sell-and-buy stop instead, since office transfers would be wiped there: sells as usual, and buys at the E price for everything except the no-buy wares and what the home town produces itself. The previous route is saved to `_backup.rou` first |
| F2 | The same, but with the 6stop office swap per target |
| F3 | A collection route instead: nothing loaded at home, one buy stop per target at the E price for everything the home town does not produce itself, everything unloaded at home |
| F4 | A trade circuit: nothing loaded at home, one self-contained trade stop per target - buy what that town produces at the R buy price, sell what it does not at the R sell price, Max amounts - and everything unloaded at home. No consumption figures and no office needed at any target; the price is the limit rather than a quantity |
| Alt + F1 / F2 | Leaves the low-value industry inputs (bricks, pig iron, pitch, hemp) out of the supplies, freeing hold space for goods with a better margin |
| Alt + F3 / F4 | Buys the no-buy wares (pitch, timber, salt, bricks, grain, hemp) as well, instead of leaving them in the town |
| Shift + F1 / F2 / F3 / F4 | Any of the above, but targeting just the currently open town and APPENDING the generated stops to the existing route instead of replacing it (combines with Alt) |

Alt is always a ware filter, never a quantity: it widens what a buying template takes and
narrows what a supplying template carries. On F1 and F2 it does not touch the buy half of
an office-less target's stop - those keep leaving the no-buy wares alone, so one press
cannot free hold space and refill it at the same time.
| F1 (goods dialog) | With the route window's goods dialog open: fill the displayed stop's empty ware slots - buy what the stop's town produces at the Ctrl+R price (Ctrl+F1 includes the no-buy wares), sell everything else at the Alt+R price, Max amounts. Existing instructions, office transfers included, stay untouched |
| DEL | Clear the selected ship's route entirely (own ships only; refused while the goods dialog is open, since it displays a stop of that route). The route is saved to `_backup.rou` first |

In the goods dialog the price keys refresh the display the way the dialog's own stop
arrows do, which starts a new Undo session: Undo covers changes made since - the same
as after a stop switch (the game resets its Undo baseline on every stop display).

Directions and amounts are never changed by the price keys; F1 is the only key that
creates orders.

The route keys take both the home town (its first stop) and the targets (the rest) from
the ship's own route; Shift and F4 use the town whose view is open, with the merchant's
home town as the source. Route
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
mod-tavern-details.)

## Known limitations

- The game prices a transaction as the average of the curve over the amount traded, not
  the marginal price at the end point, so a limit set exactly at a level lets the last
  transaction overshoot that stock point by up to one transaction chunk. Correcting for
  it needs the game's transaction chunk size, which is not reverse engineered yet.
- Log output goes to OutputDebugString (DebugView or a debugger); mod-tavern-details fakes
  the PEB BeingDebugged flag at load, which unlocks the gated logging of all mods.
