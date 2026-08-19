# mod-fix-patrol-letter-crash

Fixes a vanilla crash-to-desktop when opening the Personal letters list while it
contains certain scripted letters - most prominently the escort/patrol mission's
"Patrol destination" letters.

Install: put `fix_patrol_letter_crash.dll` into the `mods` folder (requires the
modloader).

## The bug

Every message carries a town byte that the letters list draws as its town column,
by indexing the 40-slot town-name text bank at `0x6dda00`. The lookup has no
bounds check (`0x47d928: mov eax, [edx*4+0x6dda00]`), and the resulting pointer
goes to ddraw_Dll's text draw, which dereferences it without any guard
(`ddraw_Dll+0xf100`).

The letter-creation script command (handler `0x4ed4a0`) fills that town byte with
the low byte of a script variable, unvalidated (`0x4ed4e4`). The patrol/escort
letter scripts pass a variable that is not a town index - observed values include
40, 95, 228, 242 and 255 - so drawing the list reads whatever UI global happens to
live past the bank and treats it as a string pointer:

- when the value aliases readable memory, the row draws a wrong town ("Edinburgh",
  because slot 40 is the first entry of the adjacent full town-name table) or a
  blank - this is why the bug looks intermittent;
- when it does not, the game dies on the spot, with no dump and no error dialog.

Only the list is affected: the letter body and header are formatted at creation
through a bounded town-name helper, so reading a letter is always safe.

## The fix

The unbounded lookup is detoured (7 bytes at `0x47d928`, original bytes verified
before patching): town bytes below 40 read the bank exactly as before, anything
else draws an empty string - the same blank town column the unpatched game shows
on the days it happens to survive the read. Letters whose garbage byte lands in
range (0..39) still display that town name, as they always did; there is no way
to tell those apart from genuine towns at draw time.

The buggy letter creation itself is left untouched: the intended town is not
recoverable there, and an out-of-range byte is only ever consumed by the list
draw this mod bounds.
