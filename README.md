# p3-lib

Mods for Patrician 3 (v1.1), the reverse-engineered game API they are built on, and the
tools around them.

## Usage

1. Download `dist.zip` from a [release](../../releases) and
   extract it into the game folder, the one holding `Patrician3.exe`. It brings
   `mkloadercli.exe` and `p3_modloader.dll` for the folder itself, the mods in `mods\`,
   and a corrected patrol mission script in `missions_addon\` (the game prefers a loose
   file over the copy inside `p2arch0_eng.cpr`, so nothing is overwritten).
2. Double-click `mkloadercli.exe`. It reads the `Patrician3.exe` beside it and writes
   `Patrician3_modloader.exe`, the same executable with `p3_modloader.dll` added to its
   imports. Do this once - only a reinstall or a patched game needs it again.
3. Start `Patrician3_modloader.exe` instead of `Patrician3.exe`. Every DLL in `mods\` is
   loaded before the game runs, and its `start()` is called.

The game's executable is not something this project ships, which is why the launcher is
built locally from your own copy; `mkloadercli` expects the v1.1 GOG executable and
verifies what it produces. Drop a DLL into `mods\` to add a mod, remove it to remove one.

## p3-aim

### Tests
For now, everything works only on x86:
`cargo test --target=i686-pc-windows-msvc -- --nocapture`
