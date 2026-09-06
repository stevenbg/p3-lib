//! Saving and loading the game: the two routines on the operation queue object
//! ([crate::operations::OPERATIONS_PTR]) that own the save file, their single call sites,
//! and the file operations inside them that carry the file's name.
//!
//! **Saving is an operation.** The Save button (`0x00433C1B`) and the autosave (`0x005466EA`,
//! `0x0054681E`) both enqueue opcode `0xC2` ([crate::operation::Operation::SaveGame]); the
//! queue drain's jump table `0x00547290` (index = opcode - `0xC1`) sends it to `0x005469D4`,
//! which calls the save handler. Hooking that call sees every save there is.
//!
//! **A new game is a load.** The start-game routine of the main menu's dialog (`0x004335B0`)
//! calls the load routine for a fresh game as well as for a saved one - the scenario comes
//! out of the game archive as a `.pat` - so the load's call site is the one place where the
//! whole world is replaced.

/// The save handler: `thiscall(ops, op) -> al`, `ret 4`. Picks the folder by the
/// session-mode byte ([crate::game_setup::SESSION_MODE_OFFSET]): `5` -> [SAVE_FOLDER_CAMPAIGN_ADDRESS],
/// `3` -> [SAVE_FOLDER_SINGLE_ADDRESS], below `3` -> [SAVE_FOLDER_MULTI_ADDRESS] (three
/// separate tests; `4` gets no folder), creates it (`CreateDirectoryA`), appends `\`, the
/// operation's name (up to 12 bytes, NUL-terminated) and [SAVE_EXTENSION], opens the file
/// with `CFile::Open` at [SAVE_FILE_OPEN_CALL_SITE], serialises the world (`0x00549C60`
/// sizes it, `0x00549CC0` fills the buffer), compresses (`0x0054C410`) and writes; then the
/// same for the [SAVE_COMPANION_EXTENSION] file from the object at `0x006DFC90`. Returns 0
/// once it has handled the files - after a file error too (every path through the tail
/// `0x0054767D` ends in `xor al,al`) - and 1 without writing when the operation names
/// another player's save (`op+4 >= 0` and not `ops+0x2C`); on 1 the drain reschedules the
/// autosave a minute ahead (`0x005469E0`), on 0 a full interval (`0x005469F7`).
pub const SAVE_HANDLER_ADDRESS: u32 = 0x005472F0;
/// The handler's only call: the drain's opcode `0xC2` case.
pub const SAVE_HANDLER_CALL_SITE: u32 = 0x005469D7;
/// Inside the handler, `call CFile::Open(name, SAVE_FILE_OPEN_FLAGS, 0)` for the `.pat`
/// file - `thiscall(file, name, flags, error*) -> BOOL`, `ret 0xC`. `name` is the complete
/// relative path, `Save\Ein\NAME.pat`.
pub const SAVE_FILE_OPEN_CALL_SITE: u32 = 0x0054743F;
/// MFC `CFile::Open`.
pub const CFILE_OPEN_ADDRESS: u32 = 0x00653C1D;
/// `modeCreate | modeWrite | typeBinary`.
pub const SAVE_FILE_OPEN_FLAGS: u32 = 0x9001;

/// `Save\Kam` - campaign saves.
pub const SAVE_FOLDER_CAMPAIGN_ADDRESS: *const u8 = 0x006C3860 as _;
/// `Save\Ein` - single-player saves.
pub const SAVE_FOLDER_SINGLE_ADDRESS: *const u8 = 0x006C386C as _;
/// `Save\Mehr` - multiplayer saves.
pub const SAVE_FOLDER_MULTI_ADDRESS: *const u8 = 0x006C3878 as _;
/// The save file (`0x006C3884`).
pub const SAVE_EXTENSION: &str = ".pat";
/// The second file the handler writes beside the save (`0x006C388C`), from the object at
/// `0x006DFC90`; the save dialog deletes both together (`0x00433CC0`).
pub const SAVE_COMPANION_EXTENSION: &str = ".pst";
/// The autosave operation's name: `Save\<folder>\AUTO.pat`.
pub const AUTOSAVE_NAME: &[u8] = b"AUTO";

/// The load routine: `thiscall(ops, name, expected_ticks, keep_copy, mode) -> al`,
/// `ret 0x10`. `name` is the bare file name (up to 12 bytes); `mode` picks the folder
/// through the jump table `0x00547BC4`: `0` -> `Save\Mehr\` (`0x006C38AC`), `1` and `2` ->
/// none, `3` -> `Save\Ein\` (`0x006C38A0`), `4` -> `MISSAV\` (`0x006C38B8`), `5` ->
/// `Save\Kam\` (`0x006C3894`); then `.pat` (`0x006C38C0`). The file is read through the
/// Archiver at [LOAD_READ_CALL_SITES] - first from the game archive `[0x006DCCC8]`, then
/// from disk - decompressed (`0x0054C480`) and deserialised (`0x00549F40(ops, buffer,
/// mode)`); a nonzero `expected_ticks` that differs from the loaded world's tick counter
/// (`0x006DE4B4`) fails the load. `keep_copy` keeps the compressed image at `ops+0x14`.
/// The `.pst` companion follows the same way into `0x006DFC90`.
pub const LOAD_ADDRESS: u32 = 0x005476E0;
/// The routine's only call, in the dialog's start-game routine `0x004335B0`, which stores
/// the mode into the session-mode byte afterwards (`0x0043374F`).
pub const LOAD_CALL_SITE: u32 = 0x00433742;
/// Inside the load routine, the two `call arc_CreateMemMapped(archive, name, &data, &size)`
/// for the `.pat` - cdecl, nonzero on success. The first passes the game archive, the
/// second (reached when the first fails) `0`, meaning the plain file on disk. `name` is
/// the complete relative path.
pub const LOAD_READ_CALL_SITES: [u32; 2] = [0x0054780B, 0x00547829];
/// The import thunks of `Archiver.dll`'s `arc_CreateMemMapped` and `arc_ReleaseMemMapped`.
pub const ARC_CREATE_MEM_MAPPED_ADDRESS: u32 = 0x006389EE;
pub const ARC_RELEASE_MEM_MAPPED_ADDRESS: u32 = 0x006389F4;
/// The game archive handle the loaders try before the disk.
pub const GAME_ARCHIVE_PTR_ADDRESS: *const u32 = 0x006DCCC8 as _;
