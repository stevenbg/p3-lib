# mod-auto-supply

Hotkeys for configuring a trading office administrator from live game data. All prices
are computed from the town's price thresholds and the ware base prices - no
configuration files.

Install: put `auto_supply.dll` into the `mods` folder (requires the modloader). The
hotkeys are active only while a trading office window is open, and act only on its
administrator view (the "Trading Office" side button).

## Hotkeys

| Key | Action |
|-----|--------|
| F1 | Setup: every ware without an order becomes BUY if the town produces it, SELL otherwise, at the F2 price levels. Existing orders are untouched. A buy's amount is set to 9999 only if it is currently 0. |
| F2 | Set prices of all existing orders: sells at t0 (the supply price), buys at t1. |
| F3 | Set only buy prices: t1. With ctrl: halfway t0..t1; alt: halfway t1..t2; shift: t2. |
| F4 | Set only sell prices: t0. With ctrl: halfway 0..t0; alt: halfway t0..t1; shift: t1. |
| F11 | Debug: dump thresholds, base prices and all price levels to the log and to `<TownName>.csv` in the game directory. |

Directions and amounts are never changed by the price keys; F1 is the only key that
creates orders.

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

- selling (player to town): stock 0 -> ~2.0 (trade difficulty), t0 -> 1.4, t1 -> 1.0,
  t2 -> 0.7, from t3 -> 0.5
- buying (player from town): stock 0 -> 4.0, t0 -> 1.5, t1 -> 1.0, t2 -> 0.8,
  from t3 -> 0.6

A sell limit priced at a stock point makes the administrator stop selling once the
market fills up to that point; a buy limit makes it drain the market down to it. That
is what the key levels mean: F2's defaults keep the town supplied to one week (sells
stop at t0) while buying up everything beyond t1; shift-F3 buys only the surplus
beyond t2, ctrl-F4 sells deep into scarcity, and so on.

The price levels live in `src/prices.rs` and are meant to be reused by the upcoming
in-mod trade route creation.

Details and reverse engineering notes: https://p3modding.github.io/towns/ware-prices.html

## Known limitations

- The administrator view's displayed amounts refresh only when the window is reopened;
  the underlying office data is always correct (directions and prices refresh live).
- The trade difficulty is assumed normal (`DIFFICULTY_D = 2.0`); it only affects the
  ctrl-F4 level and a small correction on low-volume wares' supply price.
- Log output goes to OutputDebugString (DebugView or a debugger); the mod fakes the
  PEB BeingDebugged flag at load, which also unlocks the gated logging of all other
  mods.
