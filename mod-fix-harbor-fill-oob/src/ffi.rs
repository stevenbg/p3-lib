//! Guards the town view's harbor flood fill against off-map coordinates.
//!
//! When a town view opens, the scene rebuilds a per-map harbor-region object
//! (`0x0062C730`): it picks seed waypoints from the map's list, pre-steps each one
//! tile diagonally with the unclamped stepper `0x0058B740`, and flood-fills the
//! region matrix from there. Neither the seed fill (`0x0062BD50`) nor the recursive
//! neighbour fill (`0x0062C0A0`) bounds-checks its coordinates, so a seed stepped
//! off the map edge indexes the matrix at a negative offset - a crash when the heap
//! places the matrix against a no-access page (observed: Ripen, convoy departing),
//! and a silent two-byte heap write (`0x0062BD8A` stores 0xFFFF) otherwise.
//!
//! The fix detours both entries to a guard that rejects any coordinate outside
//! `[0, stride) x [0, height)` (the scene's `+0xC314`/`+0xC318`). A rejected call
//! returns immediately, which is exactly what an in-range fill does when the cell
//! does not match - no game behaviour changes for in-range coordinates. Rejections
//! are logged with `warn!` and appended to `_harbor_fill_oob.log` in the game
//! folder; the file logging is temporary, to collect evidence during normal play,
//! and comes out again once the fix has soaked.
//!
//! Background: `.claude/notes/todo/town-view-harbor-fill-crash.md`.
use std::{
    arch::global_asm,
    ffi::c_void,
    io::Write,
    sync::atomic::{AtomicU32, Ordering},
};

use hooklet::windows::x86::{deploy_rel32_raw, X86Rel32Type};
use log::{error, info, warn};

/// The local map scene (town view and sea battle), and the loaded map's grid:
/// row stride at `+0xC314`, rows at `+0xC318`, map id at `+0xC324`.
const SCENE_PTR_ADDRESS: *const u32 = 0x006E51AC as _;
const SCENE_STRIDE_OFFSET: u32 = 0xC314;
const SCENE_HEIGHT_OFFSET: u32 = 0xC318;
const SCENE_MAP_ID_OFFSET: u32 = 0xC324;

/// The anchor the seed selection measures its nearest-waypoint pick from
/// (`0x0062C374` reads both as `WORD`). Logged with every rejection: it is what decides
/// whether an edge waypoint is picked at all, and it is the last unidentified input to
/// this bug.
const REGION_ANCHOR_X_OFFSET: u32 = 0x36;
const REGION_ANCHOR_Y_OFFSET: u32 = 0x38;
/// The region's `u16` matrix, so a rejection can report whether the index it would have
/// used was even outside the allocation.
const REGION_MATRIX_OFFSET: u32 = 0x59C;

/// The seed fill: `0x0062BD50(this, x, y)`, `ret 0x8`. The detour overwrites
/// `sub esp,0x10` and the start of `mov eax,[0x006E51AC]`; the stub re-executes
/// both before continuing.
const FILL_PATCH_ADDRESS: u32 = 0x0062BD50;
static FILL_CONTINUATION: u32 = 0x0062BD58;
/// The recursive neighbour fill: `0x0062C0A0(this, x, y)`, `ret 0x8`, calls itself
/// at `0x0062C2AD`. Stolen instructions: `sub esp,0x14` and `mov al,[0x0067792F]`.
const RECURSE_PATCH_ADDRESS: u32 = 0x0062C0A0;
static RECURSE_CONTINUATION: u32 = 0x0062C0A8;

/// Next to `_crash_report.txt`: the modloader's working directory is the game
/// folder. Never truncated, so hits accumulate across sessions.
const LOG_FILE: &str = "_harbor_fill_oob.log";

const SITE_NAMES: [&str; 2] = ["seed", "recurse"];

#[no_mangle]
pub unsafe extern "C" fn start() -> u32 {
    let _ = log::set_logger(&win_dbg_logger::DEBUGGER_LOGGER);
    log::set_max_level(log::LevelFilter::Info);

    if deploy_rel32_raw(FILL_PATCH_ADDRESS as _, (&fill_detour) as *const _ as _, X86Rel32Type::Jump).is_err() {
        error!("failed to detour the seed fill at {FILL_PATCH_ADDRESS:#010x}");
        return 1;
    }
    if deploy_rel32_raw(RECURSE_PATCH_ADDRESS as _, (&recurse_detour) as *const _ as _, X86Rel32Type::Jump).is_err() {
        error!("failed to detour the recursive fill at {RECURSE_PATCH_ADDRESS:#010x}");
        return 2;
    }

    info!("harbor fill guard installed on {FILL_PATCH_ADDRESS:#010x} and {RECURSE_PATCH_ADDRESS:#010x}");
    0
}

/// 1 = in range, carry on into the original function; 0 = reject, the stub returns
/// without touching the matrix. Called from the two asm stubs below with the
/// original's own arguments plus its `this`.
///
/// The log carries enough to answer two questions the static reading cannot:
///
/// - **is the recursion guard earning its place?** The neighbour deltas are `(±1, 0)` and
///   `(0, ±2)` (`0x0067B610`/`0x0067B620`) with no clamping, so a region touching any
///   border walks off it even from a perfectly valid seed. If `recurse` rejections appear,
///   guarding `0x0062C0A0` is not belt-and-braces, it is load-bearing. If only `seed` ever
///   fires, the seed guard alone would do.
/// - **which rejections are memory-unsafe, and which are only wrong?** `x` leaving
///   `[0, stride)` wraps onto the neighbouring row - a cell inside the allocation, so the
///   vanilla game reads and writes valid memory, just the wrong tile. `y` leaving
///   `[0, height)` leaves the block entirely. `inalloc` says which happened, so a guard
///   that is changing the region's *shape* (rejecting harmless row wraps that vanilla
///   performs) can be told apart from one preventing corruption.
#[no_mangle]
unsafe extern "C" fn harbor_fill_guard(site: u32, x: i32, y: i32, region: u32) -> u32 {
    let scene = *SCENE_PTR_ADDRESS;
    let (stride, height, map_id) = if scene != 0 {
        (
            *((scene + SCENE_STRIDE_OFFSET) as *const i32),
            *((scene + SCENE_HEIGHT_OFFSET) as *const i32),
            *((scene + SCENE_MAP_ID_OFFSET) as *const u32),
        )
    } else {
        // No scene: nothing sane to index, reject and record it - this path is not
        // expected to be reachable.
        (0, 0, u32::MAX)
    };
    if x >= 0 && y >= 0 && x < stride && y < height {
        return 1;
    }

    // Which bound broke, and whether the index would have left the allocation at all.
    let mut how: Vec<&str> = Vec::new();
    if x < 0 {
        how.push("x<0");
    }
    if x >= stride {
        how.push("x>=stride");
    }
    if y < 0 {
        how.push("y<0");
    }
    if y >= height {
        how.push("y>=height");
    }
    let index = stride as i64 * y as i64 + x as i64;
    let cells = stride as i64 * height as i64;
    let in_alloc = index >= 0 && index < cells;

    let site_name = SITE_NAMES.get(site as usize).copied().unwrap_or("?");
    let count = SITE_COUNTS[site.min(1) as usize].fetch_add(1, Ordering::Relaxed) + 1;
    let (anchor_x, anchor_y) = if region != 0 {
        (
            *((region + REGION_ANCHOR_X_OFFSET) as *const u16),
            *((region + REGION_ANCHOR_Y_OFFSET) as *const u16),
        )
    } else {
        (u16::MAX, u16::MAX)
    };
    let matrix = if region != 0 {
        *((region + REGION_MATRIX_OFFSET) as *const u32)
    } else {
        0
    };

    let line = format!(
        "rejected {site_name} #{count}: x={x} y={y} [{}] idx={index} inalloc={} stride={stride} height={height}          map={map_id:#04x} anchor=({anchor_x},{anchor_y}) region={region:#010x} matrix={matrix:#010x}",
        how.join(","),
        if in_alloc { "yes" } else { "NO" },
    );
    warn!("{line}");
    append_log_line(&line);
    0
}

/// Rejections so far this session, per site, so the log says at a glance whether the
/// recursion guard ever fires - which is the open question about whether it is needed.
static SITE_COUNTS: [AtomicU32; 2] = [AtomicU32::new(0), AtomicU32::new(0)];

/// Temporary evidence file - a hit is a crash or a heap corruption that did not
/// happen. Failures to write are swallowed: the guard must never be the thing
/// that breaks the frame.
fn append_log_line(line: &str) {
    let unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(LOG_FILE) {
        let _ = writeln!(file, "[unix {unix}] {line}");
    }
}

extern "C" {
    static fill_detour: c_void;
    static recurse_detour: c_void;
}

// Both stubs run at the very top of their function, before its prologue: the stack
// is [ret][x][y] and ecx is `this` - the region object, which the guard wants for the
// anchor and the matrix pointer. Only ecx must survive the guard call (the originals
// read it right after the stolen instructions); eax and edx are dead on both paths.
// A rejected call returns `ret 0x8` exactly like the originals.
//
// The guard is cdecl, so arguments go on in reverse: region, y, x, site.
global_asm!("
.global {fill_detour}
{fill_detour}:
push ecx
mov eax, dword ptr [esp + 12]
mov edx, dword ptr [esp + 8]
push ecx
push eax
push edx
push 0
call {guard}
add esp, 16
pop ecx
test eax, eax
je 2f
# the two instructions the jmp overwrote, then back into the original
sub esp, 0x10
mov eax, dword ptr [0x006E51AC]
jmp [{fill_continuation}]
2:
xor eax, eax
ret 8
",
fill_detour = sym fill_detour,
guard = sym harbor_fill_guard,
fill_continuation = sym FILL_CONTINUATION);

global_asm!("
.global {recurse_detour}
{recurse_detour}:
push ecx
mov eax, dword ptr [esp + 12]
mov edx, dword ptr [esp + 8]
push ecx
push eax
push edx
push 1
call {guard}
add esp, 16
pop ecx
test eax, eax
je 2f
# the two instructions the jmp overwrote, then back into the original
sub esp, 0x14
mov al, byte ptr [0x0067792F]
jmp [{recurse_continuation}]
2:
xor eax, eax
ret 8
",
recurse_detour = sym recurse_detour,
guard = sym harbor_fill_guard,
recurse_continuation = sym RECURSE_CONTINUATION);
