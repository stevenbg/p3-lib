# mod-fix-feeding-the-poor-wine-price

Fixes a vanilla bug in the church's "feeding the poor" donation dialog: **the wine row is
priced as salt**, so wine counts at roughly an eighth of its value toward the donation
thresholds.

Install: put `fix_feeding_the_poor_wine_price.dll` into the `mods` folder (requires the
modloader).

## The bug

The dialog values what you offer by walking its five rows - grain, meat, fish, beer, wine -
pricing each with the game's market-value routine (`0x0052E1D0`) and summing into a running
total. That total, divided by the town's beggar target
(`sqrt(citizens * poor_satisfaction / 18) + 8`), becomes the donation's **gate byte**, which
picks the reply and the effect:

|gate|reply|effect|
|-|-|-|
|`< 10`|"...will be grateful..."|reputation only|
|`10..49`|"...thank you very much for the generous donation..."|reputation only|
|`>= 50`|"An extremely generous donation! Beggars from everywhere will come..."|reputation + a one-shot **beggar influx**|

The loop uses its row counter directly as the ware id (`mov al,[ebp+0x672C14]` at
`0x005CAF83` for the barrels/loads scale, `push ebp` at `0x005CAFCE` for the price). Row
order happens to coincide with ware ids for grain (0), meat (1), fish (2) and beer (3) - but
the fifth row is wine, ware **7**, and the loop prices ware **4: salt**. The scale survives
by luck, salt and wine both being barrel wares; the price does not, wine's base being 1.1
against salt's 0.1425.

Measured in game (Luebeck, divisor 49): 65 barrels of wine scored gate **22** - the middle
reply - where correct pricing gives ~145, comfortably "extremely generous". Reaching the
influx with wine alone took ~162 barrels instead of 7.

Only the thresholds are affected. The handler (`0x004FE557`) re-prices the *delivered* goods
itself through the ware table at `0x006734CC`, so the reputation earned and the goods taken
from the warehouse were always correct - wine was merely a terrible way to trigger the
beggar influx.

## The fix

One rel32 hook on the dialog's single price call (`call 0x0052E1D0` at `0x005CAFD4`): the
hook remaps ware 4 to 7 and calls the original routine. Salt is never legitimately passed at
this site - the dialog's rows are wares 0, 1, 2, 3 and 7 - so the remap cannot misfire, and
no other caller of the price routine is touched.

Verify in game: with the fix, 7 barrels of wine in a town like Luebeck should say
"extremely generous" (and bring the beggars); without it, the same donation says
"generous" at best.
