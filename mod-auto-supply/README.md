# mod-auto-supply

Hotkeys for configuring a trading office administrator, and for editing a ship's trade
route, from live game data. All prices are computed from the game's price curves - no
configuration files.

Install: put `auto_supply.dll` into the `mods` folder (requires the modloader). Editing
routes also needs `fix_uncompressed_trade_route_loading.dll`, because generated route
files are uncompressed.

## Hotkeys

Office keys act only while a trading office window is open, on its administrator view
(the "Trading Office" side button). The others work anywhere in a running game.

| Key | Action |
|-----|--------|
| F1 | Setup: every ware without an order becomes BUY if the town produces it, SELL otherwise, at the R price levels (buy par, sell supply price). Existing orders are untouched; a buy's amount is set to 9999 only if it is currently 0. |
| Ctrl + Q W E R T Y | Set the BUY price of every ware that has a buy order, to that price level (see below) |
| Alt + Q W E R T Y | Set the SELL price of every ware that has a sell order |
| F11 | Dump thresholds, base prices and all price levels to the log and to `<TownName>.csv` in the game directory |
| F9 | Log the current town and the selected ship |
| F10 | Dump every ship's applied route chain from the route stop pool |
| Ctrl + Z | Append a stop for the current town to the selected ship's route (proof of concept: buy 10 beer at 50) |

Directions and amounts are never changed by the price keys; F1 is the only key that
creates orders.

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
(`TownPtr::get_price_thresholds()`): t0 is one week of the town's consumption, t1 adds
two more weeks (grain: four), t2 adds ten days of the town's production (verified
against `get_production_values()` across four towns - this is the gitbook's
"unidentified array"), and t3 adds another consumption week. Building materials use
fixed bases, tiny values are clamped to minimum steps, and grain/wine get seasonal
t1 bonuses in autumn.

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
