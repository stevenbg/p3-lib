# p3-rou

Library and CLI for Patrician 3 trade route files (`.rou`, stored in `Save/AutoRoute`).

The game saves routes compressed. The compression algorithm has not been reverse engineered, so generated files are written
uncompressed (negative length header). The vanilla game fails to load uncompressed routes — the
`mod-fix-uncompressed-trade-route-loading` mod fixes that and must be installed to use generated files.

## Dump a route

```
roucli dump "Save/AutoRoute/route.rou"
```

Handles both compressed (game-saved) and uncompressed (generated) files.

## Generate a route

```
roucli generate route.toml -o "Save/AutoRoute/route.rou"
```

Route description format:

```toml
[[stops]]
town = 0x0a  # town index within the savegame's town list, see below
flag = "R"   # optional stop flag as shown in the route window: "R" (repair), "X" (default), or "-"

[[stops.load]] # transfer from trading office onto the ship
ware = "fish"
amount = 2     # in-game units (Last/barrels/pieces); omit for "as much as possible"

[[stops]]
town = 0x11

[[stops.sell]] # sell to the town
ware = "fish"
amount = 2
min_price = 700

[[stops.unload]] # transfer from the ship into the trading office
ware = "fish"

[[stops.buy]] # buy from the town
ware = "beer"
amount = 20
max_price = 43
```

Ware names follow the `WareId` enum of `p3-api` (case-insensitive, spaces ignored): Grain, Meat, Fish, Beer, Salt, Honey,
Spices, Wine, Cloth, Skins, WhaleOil, Timber, IronGoods, Leather, Wool, Pitch, PigIron, Hemp, Pottery, Bricks, Sword, Bow,
Crossbow, Carbine.

Town indices are savegame-specific (towns founded during a game shift the list). To find the index of a town, save a route
that stops there in-game and inspect it with `roucli dump`.
