# p3-modloader

The DLL that loads the mods. `Patrician3_modloader.exe` is the game executable with
`p3_modloader.dll` added to its imports, so the DLL's `DllMain` runs before any game
code. It hooks the call to the game's `WinMain` (module offset `0x0023CA22`, target
`0x0064BE10`) and, when that call is made, loads every file in `mods\` with
`LoadLibrary` and calls its exported `start()` before handing over to the game.

Install: `p3_modloader.dll` next to `Patrician3_modloader.exe` in the game folder. It is
found through the executable's import table, so a copy inside `mods\` is never the one
that runs.

## Loading

Files in `mods\` are loaded in directory order. A mod is a DLL exporting
`extern "C" fn start() -> u32`; `0` means success, anything else is logged as an error
and the mod is skipped. A file without a `start` export, or one that fails to load, is
skipped the same way. The count of mods that started is logged at the end.

## Logging in debug builds

All crates log through `win_dbg_logger`, whose output reaches `OutputDebugString` (and
so DebugView) only while `IsDebuggerPresent()` is true. A **debug build** of this DLL
sets the PEB `BeingDebugged` flag in its `DllMain`, before any mod loads, so every mod's
logging - including what `start()` writes - shows up in DebugView with no debugger
attached. A **release build** does not: release builds of the mods compile their
`debug!`/`info!`/`warn!` logging away (the workspace sets `log`'s
`release_max_level_error`), so there is nothing to unlock, and a build handed to players
should not fake a debugger.
