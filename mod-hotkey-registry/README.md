# mod-hotkey-registry

The shared hotkey registry: one keyboard hook for every mod, so keys stop colliding
and window-scoped pages can claim keys that global features also use.

## Why

Before this, each mod brought its own key mechanism - `mod-trading-qol` a global
`WH_KEYBOARD` hook that sees every key everywhere, `mod-tavern-details` per-frame
polling inside its window - and cross-DLL the only arbitration was hook-chain order,
i.e. modloader load order. That is why the tavern page's keys had to retreat to
1/2/3 when its F1/F3 collided with mod-trading-qol's hook.

## What it is

Deliberately dumb: a fixed table, a LIFO dispatcher and a log. It knows nothing
about the game.

- A mod registers `(key, modifiers, handler)` and gets back a **handle**; it
  unregisters the handle when its scope closes. Scoping is entirely the consumer's
  job: register on window open / page enter, unregister on close / leave. (Every
  lifecycle hook the current mods need was play-verified first - building-window
  closes fire on every path, the goods dialog's close resets its state on every
  path, tavern page transitions are fully observable. See
  `.claude/notes/todo/hotkey-registry.md`.)
- Dispatch is **newest-first**: a window's registration shadows a global one for
  the same key while the window is open, and the global resurfaces on unregister.
- A handler returning nonzero **swallows** the keystroke (the game never sees it)
  and stops the walk; returning 0 declines and dispatch continues. Nobody handles
  -> the key passes to the game untouched.
- Handlers fire once per physical press (autorepeat and releases are filtered),
  on the game main thread, and receive the `(vk, mods)` that matched so one
  handler can serve a mod's whole key set. They may register/unregister freely -
  dispatch snapshots matches and re-checks handles before each call.
- Everything is logged by mod name: registrations, unregistrations, and who
  shadows whom.

## The ABI (version 1)

C only - no Rust types cross a DLL boundary:

```
hotkeys_api_version() -> u32
hotkeys_register(owner: *const char, vk: u32, mods: u32,
                 handler: extern "C" fn(vk: u32, mods: u32) -> u32) -> u32  // handle, 0 = failure
hotkeys_unregister(handle: u32)
```

`owner` is a static NUL-terminated mod name (safe to keep: mod DLLs never unload),
used only for the logs. `mods` is an exact-match bitmask: SHIFT = 1, CTRL = 2,
ALT = 4 - plain F1 does not fire while CTRL is held.

## Binding, from a consumer

Bind dynamically, and with `LoadLibraryW(L"mods\\hotkey_registry.dll")` rather than
`GetModuleHandleW`: the modloader starts DLLs in alphabetical order, so a consumer
that sorts earlier would otherwise look for this DLL before it is loaded.
LoadLibrary loads it on demand - registering works even before this DLL's own
`start()` has installed the hook - or just bumps the refcount. Never free it. Then
`GetProcAddress` the three exports and check `hotkeys_api_version()` against the
version the consumer was compiled for: a mismatch is one error line and inert keys,
where an unchecked signature change across the boundary would be stack corruption.
A missing hotkey_registry.dll degrades the same way - `warn!` once, keys inert, mod loads
fine.
