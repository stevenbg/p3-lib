# mod-fix-texture-cache-thrash

Fixes the framerate collapse (48 → ~17 fps) while a building window is open —
worst in the shipyard, also noticeable in the bathhouse.

## The bug

The game's graphics library (`ddraw_dll.dll`, "SGL") draws scene sprites from
decoded source images and keeps those images in an LRU cache with a hard
budget. The built-in budget is **16 MiB** — but one town view plus an open
building window needs about **19.4 MB**. The least-recently-used eviction then
removes exactly the images the next frame needs again, so the dimmed town and
the building interior are re-decoded from the archives **every frame**
(~800 ms of image decoding per second). Closing the window instantly recovers,
which is why the bug looks like "building windows are slow".

GOG evidently knew: their `gl.cfg` sets `TextureCacheSize = 48000000`, which
would be plenty. But this build of `ddraw_dll.dll` never reads `gl.cfg` — the
config parser inside the DLL has no callers, and nothing references the
file name string. The shipped setting is dead.

## The fix

One dword: the budget at `ddraw_dll.dll+0x5F734` is raised from 16 MiB to
128 MiB once the DLL is loaded. Memory use only grows to the actual working
set (a few dozen MB). The patch is applied only if the 16 MiB default is
found, so other `ddraw_dll.dll` builds are left untouched.
