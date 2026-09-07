# mkloadercli

Builds `Patrician3_modloader.exe`, the launcher that loads the mods, from the player's own
`Patrician3.exe`.

The launcher is the game's executable with one import added, `p3_modloader.dll`, so that
DLL's `DllMain` runs before any game code (see [p3-modloader](../p3-modloader/README.md)).
Being the game's executable, it is not part of this project's releases; this tool makes it
from the copy in the player's game folder.

## Usage

Copy `mkloadercli.exe` into the game folder and double-click it: it reads the
`Patrician3.exe` beside it, writes `Patrician3_modloader.exe` next to it, prints the SHA-256
and waits for Enter. From a shell the paths can be given instead:

```
mkloadercli --input "E:\GOG Galaxy\Patrician 3\Patrician3.exe" [--output ...]
```

The input must be the v1.1 GOG executable:

|File|SHA-256|
|-|-|
|`Patrician3.exe` (2,961,408 bytes)|`fe436dc5f8addc4d3aa437eb48a8bc6415d9ba5577c1027d06db9bb915eddece`|
|`Patrician3_modloader.exe` (2,965,504 bytes)|`2f960c4358df9b2a1c531bd842bf670c2ad6d1c5e6794869414708f31137bc08`|

The result is checked against the second hash before it is written; a mismatch is an
error, not a file. `--force` patches an executable with a different hash, unverified.

## The patch

- A section `.mod` (one file page, 64 KiB virtual, read-write data) is appended, holding a
  copy of the import directory, one more descriptor naming `p3_modloader` with an empty
  thunk list - Windows loads a DLL named by a descriptor even when nothing is imported
  from it - the terminating descriptor, and the name.
- The import data directory points at that copy.
- The section count and `SizeOfImage` grow to match.

Nothing else changes: the code, data and resources are the original bytes, and so is the
checksum field, which Windows does not verify for an executable.
