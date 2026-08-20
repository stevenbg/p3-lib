# mod-ui-tweaks

Small quality-of-life tweaks to the game's own UI, independent of any trading
automation.

Install: put `ui_tweaks.dll` into the `mods` folder (requires the modloader).

## Tweaks

- **Letter popups name their town**: the incoming-letter notifications on the top
  right read "Personal letter: Patrol - Stockholm" instead of "Personal letter:
  Patrol", so mission letters say at a glance where to send the ship. For
  scripted letters (whose town field is broken - see mod-fix-patrol-letter-crash)
  the town is recovered from the letter text: the last town name the letter
  mentions.

## Development

This mod also fakes the PEB BeingDebugged flag at load, which unlocks the gated
win_dbg_logger output of every mod for DebugView. Remove that here when it is no
longer wanted.
