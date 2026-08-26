# mod-fix-siege-beggar-satisfaction-bonus

Fixes a vanilla bug that permanently raises a town's **beggar satisfaction** every time it
repels a siege, so the town attracts more and more beggars for the rest of the game.

Install: put `fix_siege_beggar_satisfaction_bonus.dll` into the `mods` folder (requires the
modloader).

## The bug

A town's satisfactions live in the array at `town + 0x300`, one signed `i16` per population
type: rich, wealthy, poor - and beggars, at `+0x306`.

When a besieged town drives its attackers off, the siege tick hands out a flat bonus of 20:

```asm
00629a32  mov  edx, 4                                ; the loop counter
00629a37  xor  eax, eax
00629a39  mov  al, [esi+0x5d3]                       ; town index
...
00629a50  add  WORD PTR [ebx+eax*2+0x300], 0x14      ; satisfaction += 20
00629a60  inc  ecx
00629a61  dec  edx
00629a62  jne  0x629a37
```

`ecx` walks the array `0, 1, 2, 3`, so **four** iterations cover the three real classes and
then the beggars.

For rich, wealthy and poor that is merely a jump the game undoes on its own: it bypasses the
normal step size, but `update_citizen_satisfaction` (`0x0051C830`) recomputes those three
every day and walks them back toward what the town actually deserves.

Beggars have no such maintenance. That routine calculates every population type **except**
beggars, so nothing anywhere decays `+0x306` again - the `+20` is permanent, and it stacks
once per siege repelled. Measured across 24 towns, the field reads `-20` in the 21 that have
never been besieged and `60` in the three that have.

It matters because beggar satisfaction is the only driver of a town's beggar target, and the
target scales with the square root of population times satisfaction. So each siege a town
survives leaves it permanently more attractive to beggars, and the effect compounds.

## The fix

One byte. `0x00629A33` is the immediate of `mov edx, 4` (the `0xBA` opcode sits at
`0x00629A32`), and the mod writes `3` over it:

```
mov edx, 4   ->   mov edx, 3
```

Three iterations instead of four. Rich, wealthy and poor still get their bonus for holding
the town; the beggar slot is left alone, which is almost certainly what was intended - the
array simply has a fourth entry that the loop counted without meaning to.

No detour and no new code, so nothing has to be relocated. The `VirtualProtect` call asks for
five bytes only because that is the length of the whole instruction; just the one byte is
written, and the original page protection is restored afterwards.

What this does **not** do is repair towns that have already banked the bonus. The inflated
value is part of the savegame, so a town that repelled sieges before the mod was installed
keeps its raised beggar satisfaction; the fix stops it growing any further.
