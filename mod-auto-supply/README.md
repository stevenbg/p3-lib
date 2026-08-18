# mod-auto-supply

Hotkeys for configuring a trading office administrator, and for editing a ship's trade
route, from live game data. All prices are computed from the game's price curves - no
configuration files.

Install: put `auto_supply.dll` into the `mods` folder (requires the modloader). Editing
routes also needs `fix_uncompressed_trade_route_loading.dll`, because generated route
files are uncompressed.

## Hotkeys

F1 and the price keys act only while a trading office window is open, on its
administrator view (the "Trading Office" side button). The others work anywhere in a
running game, and take the town whose view is open (on the world map, the town last
visited).

| Key | Action |
|-----|--------|
| F1 | Setup: every ware without an order becomes BUY if the town produces it, SELL otherwise, at the R price levels (buy par, sell supply price). Existing orders are untouched; a buy's amount is set to 9999 only if it is currently 0. |
| Ctrl + F1 | Provision the celebration goods (beer, wine, fish, meat, grain, honey - a celebration needs them in stock): raise their amounts to a week of the town's citizen consumption - never lowering - and lock their quantities ("Lock min. store quantity for auto trade ships"), so route ships cannot take the stock. Directions and prices untouched. |
| Ctrl + Q W E R T Y | Set the BUY price of every ware that has a buy order, to that price level (see below) |
| Alt + Q W E R T Y | Set the SELL price of every ware that has a sell order |
| F11 | Dump thresholds, base prices, all price levels and the weekly citizen/business consumptions to the log and to `<TownName>.csv` in the game directory |
| F3 | Replace the selected ship's route with a 5stop supply route: load a week of the current town's demand at the home office, sell it there, reset that office's stock to the same amounts and haul the surplus home. Alt+F3 uses the 6stop variant, Ctrl+F3 a suck route parked in the current town. The previous route is saved to `_backup.rou` first |
| F4 | Append a trade stop for the current town to the selected ship's route: buy what the town produces at the Ctrl+Y price, sell everything else at the Alt+Y price, sells listed above the buys. Pitch, timber, salt, bricks, grain and hemp are never bought - their margin does not pay for the cargo space early on |
| Ctrl + F4 | The same stop, but buying every ware the town produces |
| F9 | Log the current town and the selected ship |
| F10 | Dump every ship's applied route chain from the route stop pool |

Directions and amounts are never changed by the price keys; F1 is the only key that
creates orders.

The route keys take the town whose view is open as the target and the merchant's home
town (the home office) as the source. Route quantities are one week of the target
town's real consumption - citizens plus businesses, the market hall consumption
window's Total column, read live from the town - rounded up to whole in-game units;
prices are the R levels. The t0 threshold is deliberately not used for quantities: it
is a comfortable stock level, inflated by minimum floors and construction reserves, not
what disappears weekly. Wares the town produces itself are not supplied to it, and
neither are bricks, pig iron, pitch and hemp - low-value industry inputs not worth the
hold space; whatever the target office holds of them is still hauled home. Route
stops that transfer wares to or from an office are wiped by the game in towns where
there is no player office, and the keys warn when that applies.

Stops load and buy the barrel goods before the bulky loads goods, each group ordered by
ware value with the best first, so when hold space runs out the least valuable cargo is
what gets left behind (see p3-rou's README).

## Price levels

Q to Y ascend in price. Each key names a point on the price curve, expressed as a
fraction between two of the town's price thresholds:

| key | BUY (ctrl) | factor | SELL (alt) | factor |
|-----|------------|--------|------------|--------|
| Q | t2 | 0.80 | t1 | 1.00 |
| W | mid t1..t2 | 0.90 | mid t0..t1 | 1.20 |
| E | 30% t1->t2 | 0.94 | 30% t0->t1 | 1.28 |
| R | t1 (par) | 1.00 | t0 (supply price) | 1.40 |
| T | 70% t0->t1 | 1.15 | 70% 0->t0 | 1.58 |
| Y | mid t0..t1 | 1.25 | mid 0..t0 | 1.70 |

A sell limit priced at a stock point makes the administrator stop selling once the
market fills up to that point; a buy limit makes it drain the market down to it. So
buying gets more aggressive towards Y, selling more restrained, and R is the natural
pair: buy at par, sell at the price that leaves the town one week of supply.

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

## Known limitations

- The administrator view's displayed amounts refresh only when the window is reopened;
  the underlying office data is always correct (directions and prices refresh live).
- The game prices a transaction as the average of the curve over the amount traded, not
  the marginal price at the end point, so a limit set exactly at a level lets the last
  transaction overshoot that stock point by up to one transaction chunk. Correcting for
  it needs the game's transaction chunk size, which is not reverse engineered yet.
- Log output goes to OutputDebugString (DebugView or a debugger); the mod fakes the PEB
  BeingDebugged flag at load, which also unlocks the gated logging of all other mods.
