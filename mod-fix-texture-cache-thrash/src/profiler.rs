//! The graphics profiler that found the texture cache thrash. It is DEAD CODE - nothing
//! in this mod calls any of it - but it is the only tooling that can measure the bug and
//! prove a fix, so it is kept whole next to the fix instead of rotting in a throwaway
//! debug hotkey.
//!
//! To bring it back, call [`install`] from `start()` (which runs on the game's main
//! thread, the thread the sampler profiles) and [`set_enabled`] from a hotkey:
//!
//! ```ignore
//! if let Err(code) = profiler::install() {
//!     return code;
//! }
//! // ... later, from a key handler:
//! profiler::set_enabled(true);
//! ```
//!
//! While enabled it writes one block per second to DebugView and to `_perf_probe.log` in
//! the game folder (truncated on each enable, so a run can be grepped from WSL
//! afterwards). [`toggle_cache_budget`] and [`set_no_dim`] are the two runtime
//! experiments the diagnosis needed.
//!
//! What the block reports, per second: frames and the worst frame delta; the cache usage
//! against its budget; screen-area submissions with their pixel area, tallied per calling
//! site; time inside the layer-select and rect-blit thunks; AIM decode time with a
//! per-file and per-asset tally and the exe/library sites that triggered each decode;
//! texture frees, graphic-record destructions and variant switches with their call sites;
//! the shipyard window's own update and draw cost; and a sampling profile of the main
//! thread's hottest 256-byte code regions, resolved to module+rva.
//!
//! Keep extra speed at x1 while measuring: mod-ui-tweaks scales the frame delta at
//! `[0x6DCCF4]` that the frame counter reads.
#![allow(dead_code)]

use std::mem;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, Ordering};
use std::sync::Mutex;

use hooklet::windows::x86::{
    deploy_rel32_raw, hook_call_rel32_with_module, hook_function_pointer, hook_function_pointer_width_module, CallRel32Hook, FunctionPointerHook, X86Rel32Type,
};
use log::{debug, error, info};
use windows::core::s;
use windows::Win32::System::Diagnostics::Debug::{GetThreadContext, CONTEXT};
use windows::Win32::System::LibraryLoader::{
    GetModuleFileNameA, GetModuleHandleA, GetModuleHandleExA, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
};
use windows::Win32::System::Memory::{VirtualQuery, MEMORY_BASIC_INFORMATION, MEM_COMMIT};
use windows::Win32::System::Threading::{GetCurrentThreadId, OpenThread, ResumeThread, SuspendThread, THREAD_GET_CONTEXT, THREAD_SUSPEND_RESUME};

use crate::ffi::CACHE_LIMIT_RVA;

/// The library's usage counter - the bytes of decoded images it currently holds, which
/// the evictors compare against the budget at [`CACHE_LIMIT_RVA`].
const CACHE_USAGE_RVA: u32 = 0x80F14;
/// The library's code, as an RVA range: used to attribute a decode to the sgl internal
/// that started it.
const DDRAW_CODE_RVAS: std::ops::Range<u32> = 0x1000..0x50000;
/// The executable's .text, for the same attribution on the game's side. Fixed, because
/// the exe has an empty relocation table and always loads at `0x400000`.
const EXE_CODE: std::ops::Range<u32> = 0x00401000..0x0066A000;

/// ddraw_dll.dll's load base, resolved once and then cached; every RVA above is relative
/// to it.
static DDRAW_BASE: AtomicU32 = AtomicU32::new(0);

unsafe fn ddraw_base() -> u32 {
    let cached = DDRAW_BASE.load(Ordering::Relaxed);
    if cached != 0 {
        return cached;
    }
    let base = match GetModuleHandleA(s!("ddraw_dll.dll")) {
        Ok(module) if !module.is_invalid() => module.0 as u32,
        _ => return 0,
    };
    DDRAW_BASE.store(base, Ordering::Relaxed);
    base
}

/// Read one of the library's dwords, or 0 while it is not loaded.
unsafe fn ddraw_dword(rva: u32) -> u32 {
    let base = ddraw_base();
    if base == 0 {
        0
    } else {
        *((base + rva) as *const u32)
    }
}

/// Whether the per-second reporting is on; see [`set_enabled`]. The counting hooks
/// themselves always run once [`install`] succeeded - they are cheap.
static PERF_PROBE_ON: AtomicBool = AtomicBool::new(false);

/// Perf probe lines also go to _perf_probe.log in the game folder (truncated on each
/// enable), so a run can be grepped from WSL afterwards instead of pasted.
static PERF_LOG: Mutex<Option<std::fs::File>> = Mutex::new(None);

fn perf_log_write(line: &str) {
    use std::io::Write;
    if let Ok(mut guard) = PERF_LOG.lock() {
        if let Some(file) = guard.as_mut() {
            let _ = writeln!(file, "{line}");
        }
    }
}

macro_rules! perf_log {
    ($($arg:tt)*) => {{
        let line = format!($($arg)*);
        log::debug!("{}", line);
        perf_log_write(&line);
    }};
}

struct PerfState {
    frames: u32,
    delta_max: u32,
    submits: u32,
    area: u64,
    /// (return address, calls, area) per submission site, this bucket.
    sites: Vec<(u32, u32, u64)>,
    sy_update_tsc: u64,
    sy_update_calls: u32,
    sy_draw_tsc: u64,
    sy_draw_calls: u32,
    /// Time inside the two ddraw thunks the submitter calls per rect per layer:
    /// layer select `0x4BB780` = `jmp [0x6DA928]`, rect blit `0x4BB140` = `jmp [0x6DAA8C]`.
    select_tsc: u64,
    select_calls: u32,
    blit_tsc: u64,
    blit_calls: u32,
    /// AIM image decodes through ddraw_dll's AIM_CONVERT_MEMFILE import, with a
    /// per-compressed-size tally - the size identifies the asset (e.g. the shipyard
    /// water strip is ~1204 KB).
    convert_tsc: u64,
    convert_calls: u32,
    convert_bytes: u64,
    converts: Vec<(u32, u32)>,
    /// Per-file decode tally from AIM.dll's internal decode_supported_files call site
    /// (aim.dll+0x2984, the same one mod-high-res hooks): (file name, calls, tsc).
    decodes: Vec<(String, u32, u64)>,
    /// Variant switches on multi-variant graphic records (0x46B710): each switch frees
    /// ALL the record's loaded textures and reloads the new variant. (record, count).
    variant_switches: u32,
    variant_records: Vec<(u32, u32)>,
    /// Which exe code triggered each runtime decode: the first return address in the
    /// exe's .text found by scanning the stack from the decode hook. (site, count).
    decode_sites: Vec<(u32, u32)>,
    /// sgl_FreeMemoryTexture calls and their exe call sites (52 static callers).
    free_calls: u32,
    free_sites: Vec<(u32, u32)>,
    /// Graphic-record destructor (0x4B3900, class vtable 0x670B28) calls and the
    /// exe sites that triggered them - the window-open mass destruction of ~304
    /// scene records should name its destroyer here.
    dtor_calls: u32,
    dtor_sites: Vec<(u32, u32)>,
    /// Per-frame tracking of the graphic-def texture handles ([0x6E2E70] array,
    /// def+0x34): (def, handle) snapshot of the previous frame, change tally.
    prev_handles: Vec<(u32, u32)>,
    validated_defs: Vec<u32>,
    handle_changes: u32,
    handle_change_frames: u32,
    changed_defs: Vec<(u32, u32)>,
    /// First ddraw_dll.dll return address on the decode stack: names the sgl internal
    /// (and thus the export) that initiates each runtime decode.
    decode_dll_sites: Vec<(u32, u32)>,
    bucket_start_ms: u32,
    bucket_start_tsc: u64,
}

static PERF: Mutex<PerfState> = Mutex::new(PerfState {
    frames: 0,
    delta_max: 0,
    submits: 0,
    area: 0,
    sites: Vec::new(),
    sy_update_tsc: 0,
    sy_update_calls: 0,
    sy_draw_tsc: 0,
    sy_draw_calls: 0,
    select_tsc: 0,
    select_calls: 0,
    blit_tsc: 0,
    blit_calls: 0,
    convert_tsc: 0,
    convert_calls: 0,
    convert_bytes: 0,
    converts: Vec::new(),
    decodes: Vec::new(),
    variant_switches: 0,
    variant_records: Vec::new(),
    decode_sites: Vec::new(),
    free_calls: 0,
    free_sites: Vec::new(),
    dtor_calls: 0,
    dtor_sites: Vec::new(),
    prev_handles: Vec::new(),
    validated_defs: Vec::new(),
    handle_changes: 0,
    handle_change_frames: 0,
    changed_defs: Vec::new(),
    decode_dll_sites: Vec::new(),
    bucket_start_ms: 0,
    bucket_start_tsc: 0,
});

const FRAME_UPDATER_ADDRESS: u32 = 0x004BD180;
const FRAME_UPDATER_ORIGINAL: [u8; 6] = [0x8b, 0x0d, 0xec, 0xcc, 0x6d, 0x00];
static FRAME_PROBE_CONTINUATION: u32 = 0x004BD186;
const SUBMIT_AREA_ADDRESS: u32 = 0x004B9650;
const SUBMIT_AREA_ORIGINAL: [u8; 5] = [0xa1, 0x94, 0xcb, 0x6d, 0x00];
static SUBMIT_PROBE_CONTINUATION: u32 = 0x004B9655;
/// Module-relative offsets of the shipyard window's vtable slots (vtable 0x0067A798).
const SHIPYARD_UPDATE_VTABLE_OFFSET: u32 = 0x27A88C;
const SHIPYARD_DRAW_VTABLE_OFFSET: u32 = 0x27A834;
static SY_UPDATE_HOOK_PTR: AtomicPtr<FunctionPointerHook> = AtomicPtr::new(std::ptr::null_mut());
static SY_DRAW_HOOK_PTR: AtomicPtr<FunctionPointerHook> = AtomicPtr::new(std::ptr::null_mut());
/// Module-relative offsets of the ddraw thunk pointers (`jmp [ptr]` at 0x4BB780/0x4BB140).
const DDRAW_SELECT_POINTER_OFFSET: u32 = 0x2DA928;
const DDRAW_BLIT_POINTER_OFFSET: u32 = 0x2DAA8C;
static DDRAW_SELECT_HOOK_PTR: AtomicPtr<FunctionPointerHook> = AtomicPtr::new(std::ptr::null_mut());
static DDRAW_BLIT_HOOK_PTR: AtomicPtr<FunctionPointerHook> = AtomicPtr::new(std::ptr::null_mut());
/// ddraw_dll.dll's IAT slot for AIM.dll's `?AIM_CONVERT_MEMFILE@@YAHPAUAIM_IMAGE@@PBDPAEJ@Z`
/// (module-relative; the AIM thunk block starts at +0x50024, CONVERT_MEMFILE is second;
/// +0x50030 = AIM_FREE and +0x5003C = AIM_INIT for a future memoizing fix).
const AIM_CONVERT_MEMFILE_IAT_OFFSET: u32 = 0x50028;
static AIM_CONVERT_HOOK_PTR: AtomicPtr<FunctionPointerHook> = AtomicPtr::new(std::ptr::null_mut());
/// ddraw_dll's IAT slot for `?AIM_CONVERT_RAW@@YAHPAUAIM_IMAGE@@@Z` - the suspected
/// per-frame decode entry (CONVERT_MEMFILE measured zero calls at runtime).
const AIM_CONVERT_RAW_IAT_OFFSET: u32 = 0x50034;
static AIM_CONVERT_RAW_HOOK_PTR: AtomicPtr<FunctionPointerHook> = AtomicPtr::new(std::ptr::null_mut());
/// The exe's binding slot for sgl_FreeMemoryTexture (thunk 0x4BB490, slot 0x6DA9DC,
/// module-relative 0x2DA9DC): the texture free path with 52 static callers.
const FREE_MEMORY_TEXTURE_SLOT_OFFSET: u32 = 0x2DA9DC;
static FREE_MEMORY_TEXTURE_HOOK_PTR: AtomicPtr<FunctionPointerHook> = AtomicPtr::new(std::ptr::null_mut());
/// sgl_SetConstantColor's binding slot (thunk 0x4BB870 = jmp [0x6DA8EC]). The no-dim
/// experiment: translucent-white constant colors (the parchment's 0x64FFFFFF dim and
/// the per-sprite alpha dims) force the modulated blit path, which cannot use the
/// cached surface and transiently re-decodes the source image EVERY FRAME - the
/// decode storm. [`set_no_dim`] forces such colors to opaque white to A/B this.
const SET_CONSTANT_COLOR_SLOT_OFFSET: u32 = 0x2DA8EC;
static SET_CONSTANT_COLOR_HOOK_PTR: AtomicPtr<FunctionPointerHook> = AtomicPtr::new(std::ptr::null_mut());
static NO_DIM: AtomicBool = AtomicBool::new(false);

/// Run the no-dim experiment: force every translucent-white constant color to opaque
/// white, which short-circuits the modulated blit path (expect graphical artifacts).
pub unsafe fn set_no_dim(on: bool) {
    NO_DIM.store(on, Ordering::SeqCst);
}

/// The call site inside AIM.dll where every file decode passes with its PATH
/// (cdecl (image_inner, file_path, file_data, file_size) -> i32). Same site
/// mod-high-res hooks for image substitution; chaining is fine.
const AIM_DECODE_FILE_CALL_OFFSET: u32 = 0x2984;
static AIM_DECODE_FILE_HOOK_PTR: AtomicPtr<CallRel32Hook> = AtomicPtr::new(std::ptr::null_mut());
/// The graphic-record class destructor (vtable 0x670B28 slot 0). Prologue is
/// push esi / mov esi,ecx / call 0x4B2D40; the detour replays the two moves,
/// re-emits the inner call indirectly, and continues at 0x4B3908.
const RECORD_DTOR_ADDRESS: u32 = 0x004B3900;
const RECORD_DTOR_ORIGINAL: [u8; 5] = [0x56, 0x8b, 0xf1, 0xe8, 0x38];
static RECORD_DTOR_INNER: u32 = 0x004B2D40;
static RECORD_DTOR_CONTINUATION: u32 = 0x004B3908;

/// The graphic-record variant switch (thiscall(record; new_variant)): frees all loaded
/// frame textures (loop over [record+0x4] x byte [record+0x20] via sgl_FreeTexture) and
/// reloads when byte [record+0x38] (current variant) differs from the argument.
const VARIANT_SWITCH_ADDRESS: u32 = 0x0046B710;
const VARIANT_SWITCH_ORIGINAL: [u8; 6] = [0x83, 0xec, 0x1c, 0x33, 0xc0, 0x53];
static VARIANT_SWITCH_CONTINUATION: u32 = 0x0046B716;

extern "C" {
    static frame_probe_stub: core::ffi::c_void;
    static submit_probe_stub: core::ffi::c_void;
    static variant_switch_probe_stub: core::ffi::c_void;
    static record_dtor_probe_stub: core::ffi::c_void;
}

// Both stubs sit on a function ENTRY: replay the stolen prologue bytes and jump to the
// continuation, preserving everything around the Rust call. In the submit stub the
// pushfd+pushad displace esp by 0x24, so the caller's return address (= the submitting
// site) is at esp+0x24 and the rect argument at esp+0x28.
std::arch::global_asm!("
.global {dtor_stub}
{dtor_stub}:
pushfd
pushad
mov eax, dword ptr [esp + 0x18]
push eax
call {on_dtor}
add esp, 4
popad
popfd
push esi
mov esi, ecx
call [{dtor_inner}]
jmp [{dtor_cont}]

.global {variant_stub}
{variant_stub}:
pushfd
pushad
mov eax, dword ptr [esp + 0x18]
mov edx, dword ptr [esp + 0x28]
push edx
push eax
call {on_variant}
add esp, 8
popad
popfd
sub esp, 0x1c
xor eax, eax
push ebx
jmp [{variant_cont}]

.global {frame_stub}
{frame_stub}:
pushfd
pushad
call {on_frame}
popad
popfd
mov ecx, dword ptr [0x6dccec]
jmp [{frame_cont}]

.global {submit_stub}
{submit_stub}:
pushfd
pushad
mov eax, dword ptr [esp + 0x24]
mov edx, dword ptr [esp + 0x28]
push edx
push eax
call {on_submit}
add esp, 8
popad
popfd
mov eax, dword ptr [0x6dcb94]
jmp [{submit_cont}]
",
    dtor_stub = sym record_dtor_probe_stub,
    on_dtor = sym on_perf_record_dtor,
    dtor_inner = sym RECORD_DTOR_INNER,
    dtor_cont = sym RECORD_DTOR_CONTINUATION,
    variant_stub = sym variant_switch_probe_stub,
    on_variant = sym on_perf_variant_switch,
    variant_cont = sym VARIANT_SWITCH_CONTINUATION,
    frame_stub = sym frame_probe_stub,
    on_frame = sym on_perf_frame,
    frame_cont = sym FRAME_PROBE_CONTINUATION,
    submit_stub = sym submit_probe_stub,
    on_submit = sym on_perf_submit,
    submit_cont = sym SUBMIT_PROBE_CONTINUATION,
);

/// Install the exe-side hooks. Call from `start()`, on the game's main thread - that is
/// the thread the sampling profiler this also starts will suspend and sample.
///
/// - an entry detour on the frame clock updater `0x004BD180` (6-byte prologue
///   `mov ecx,[0x6DCCEC]`, position-independent) counts frames, tracks the worst
///   frame delta (`[0x6DCCF4]`, last frame's ms) and closes a stats bucket once per
///   OS second (`[0x6DCCF0]`);
/// - an entry detour on the screen-area submitter `0x004B9650` (5-byte prologue
///   `mov eax,[0x6DCB94]`) counts every submission, sums the submitted pixel area
///   and tallies both per RETURN ADDRESS, so the log names the code that floods the
///   renderer;
/// - entry detours on the graphic-record destructor `0x004B3900` and the variant
///   switch `0x0046B710`, the two mass-free paths a window open was suspected of;
/// - `hook_function_pointer` wrappers on the shipyard window's update (+0xF4,
///   vtable slot 0x0067A88C) and draw (+0x9C, slot 0x0067A834) time the window's own
///   code with rdtsc, converted to ms via the bucket's measured cycles-per-ms, and on
///   the layer-select and rect-blit thunks.
///
/// The hooks into the other modules wait for the first [`set_enabled`].
pub unsafe fn install() -> Result<(), u32> {
    let frame_bytes = std::slice::from_raw_parts(FRAME_UPDATER_ADDRESS as *const u8, 6);
    if frame_bytes != FRAME_UPDATER_ORIGINAL {
        error!("perf probe: unexpected bytes at the frame updater: {frame_bytes:02x?}");
        return Err(5);
    }
    let submit_bytes = std::slice::from_raw_parts(SUBMIT_AREA_ADDRESS as *const u8, 5);
    if submit_bytes != SUBMIT_AREA_ORIGINAL {
        error!("perf probe: unexpected bytes at the area submitter: {submit_bytes:02x?}");
        return Err(6);
    }
    if deploy_rel32_raw(FRAME_UPDATER_ADDRESS as _, &frame_probe_stub as *const _ as _, X86Rel32Type::Jump).is_err() {
        error!("perf probe: deploying the frame detour failed");
        return Err(5);
    }
    if deploy_rel32_raw(SUBMIT_AREA_ADDRESS as _, &submit_probe_stub as *const _ as _, X86Rel32Type::Jump).is_err() {
        error!("perf probe: deploying the submit detour failed");
        return Err(6);
    }
    let dtor_bytes = std::slice::from_raw_parts(RECORD_DTOR_ADDRESS as *const u8, 5);
    if dtor_bytes != RECORD_DTOR_ORIGINAL {
        error!("perf probe: unexpected bytes at the record dtor: {dtor_bytes:02x?}");
        return Err(15);
    }
    if deploy_rel32_raw(RECORD_DTOR_ADDRESS as _, &record_dtor_probe_stub as *const _ as _, X86Rel32Type::Jump).is_err() {
        error!("perf probe: deploying the record dtor detour failed");
        return Err(15);
    }
    let variant_bytes = std::slice::from_raw_parts(VARIANT_SWITCH_ADDRESS as *const u8, 6);
    if variant_bytes != VARIANT_SWITCH_ORIGINAL {
        error!("perf probe: unexpected bytes at the variant switch: {variant_bytes:02x?}");
        return Err(14);
    }
    if deploy_rel32_raw(VARIANT_SWITCH_ADDRESS as _, &variant_switch_probe_stub as *const _ as _, X86Rel32Type::Jump).is_err() {
        error!("perf probe: deploying the variant switch detour failed");
        return Err(14);
    }
    match hook_function_pointer(SHIPYARD_UPDATE_VTABLE_OFFSET, shipyard_update_perf_hook as usize as u32) {
        Ok(hook) => SY_UPDATE_HOOK_PTR.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
        Err(_) => {
            error!("perf probe: hooking shipyard update failed");
            return Err(7);
        }
    }
    match hook_function_pointer(SHIPYARD_DRAW_VTABLE_OFFSET, shipyard_draw_perf_hook as usize as u32) {
        Ok(hook) => SY_DRAW_HOOK_PTR.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
        Err(_) => {
            error!("perf probe: hooking shipyard draw failed");
            return Err(8);
        }
    }
    match hook_function_pointer(DDRAW_SELECT_POINTER_OFFSET, ddraw_select_perf_hook as usize as u32) {
        Ok(hook) => DDRAW_SELECT_HOOK_PTR.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
        Err(_) => {
            error!("perf probe: hooking the ddraw select thunk failed");
            return Err(9);
        }
    }
    match hook_function_pointer(DDRAW_BLIT_POINTER_OFFSET, ddraw_blit_perf_hook as usize as u32) {
        Ok(hook) => DDRAW_BLIT_HOOK_PTR.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
        Err(_) => {
            error!("perf probe: hooking the ddraw blit thunk failed");
            return Err(10);
        }
    }
    // The sampler needs the main thread's id, and start() runs on it.
    MAIN_THREAD_ID.store(GetCurrentThreadId(), Ordering::SeqCst);
    start_perf_sampler();
    Ok(())
}

/// The probe hooks into other modules (ddraw_dll.dll, aim.dll), installed lazily on the
/// first [`set_enabled`]: at `start()` time the modloader may run before those DLLs are
/// loaded, and a failed install must never take the rest of the probe down with it.
/// Each failure is logged and skipped.
unsafe fn install_perf_late_hooks() {
    if !AIM_CONVERT_HOOK_PTR.load(Ordering::SeqCst).is_null() {
        return;
    }
    match hook_function_pointer_width_module(
        s!("ddraw_dll.dll"),
        AIM_CONVERT_MEMFILE_IAT_OFFSET,
        aim_convert_perf_hook as usize as u32,
    ) {
        Ok(hook) => AIM_CONVERT_HOOK_PTR.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
        Err(e) => error!("perf probe: hooking AIM_CONVERT_MEMFILE failed: {e:?}"),
    }
    match hook_function_pointer_width_module(
        s!("ddraw_dll.dll"),
        AIM_CONVERT_RAW_IAT_OFFSET,
        aim_convert_raw_perf_hook as usize as u32,
    ) {
        Ok(hook) => AIM_CONVERT_RAW_HOOK_PTR.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
        Err(e) => error!("perf probe: hooking AIM_CONVERT_RAW failed: {e:?}"),
    }
    match hook_call_rel32_with_module("aim.dll", AIM_DECODE_FILE_CALL_OFFSET, aim_decode_file_perf_hook as usize as u32) {
        Ok(hook) => AIM_DECODE_FILE_HOOK_PTR.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
        Err(e) => error!("perf probe: hooking aim.dll decode_supported_files failed: {e:?}"),
    }
    // The sgl binding slots are filled by the exe's own binder during graphics init,
    // long after start() - hooking on the first enable is safe from being overwritten.
    match hook_function_pointer(FREE_MEMORY_TEXTURE_SLOT_OFFSET, free_memory_texture_perf_hook as usize as u32) {
        Ok(hook) => FREE_MEMORY_TEXTURE_HOOK_PTR.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
        Err(e) => error!("perf probe: hooking sgl_FreeMemoryTexture failed: {e:?}"),
    }
    match hook_function_pointer(SET_CONSTANT_COLOR_SLOT_OFFSET, set_constant_color_hook as usize as u32) {
        Ok(hook) => SET_CONSTANT_COLOR_HOOK_PTR.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst),
        Err(e) => error!("perf probe: hooking sgl_SetConstantColor failed: {e:?}"),
    }
    info!(
        "perf probe late hooks: convert_memfile {} convert_raw {} decode_file {}",
        if AIM_CONVERT_HOOK_PTR.load(Ordering::SeqCst).is_null() { "MISSING" } else { "ok" },
        if AIM_CONVERT_RAW_HOOK_PTR.load(Ordering::SeqCst).is_null() { "MISSING" } else { "ok" },
        if AIM_DECODE_FILE_HOOK_PTR.load(Ordering::SeqCst).is_null() { "MISSING" } else { "ok" },
    );
}

/// Turn the per-second reporting on or off. The first enable installs the hooks into
/// ddraw_dll.dll and aim.dll and truncates `_perf_probe.log`.
pub unsafe fn set_enabled(on: bool) {
    if on {
        install_perf_late_hooks();
        dump_scene_graphic_vtables();
        *PERF_LOG.lock().unwrap() = std::fs::File::create("_perf_probe.log").ok();
    } else {
        *PERF_LOG.lock().unwrap() = None;
    }
    PERF_PROBE_ON.store(on, Ordering::SeqCst);
    info!("perf probe: {}", if on { "on" } else { "off" });
}

/// One-shot on enable: walk the local-map scene's graphic-object table
/// (scene [0x6E51AC] + 0x3FC, indexed by tile WORD ids) and tally the object
/// vtables, to identify the iso-graphic class(es) that re-decode per frame.
unsafe fn dump_scene_graphic_vtables() {
    let scene = *(0x006E51AC as *const u32);
    if scene == 0 {
        info!("scene gfx: no scene window");
        return;
    }
    let table = *((scene + 0x3FC) as *const u32);
    if table == 0 || !is_readable_dword(table) {
        info!("scene gfx: no object table");
        return;
    }
    let mut tallies: Vec<(u32, u32)> = Vec::new();
    let mut invalid_run = 0u32;
    let mut index = 0u32;
    let mut objects = 0u32;
    while index < 8192 && invalid_run < 512 {
        let slot = table + index * 4;
        index += 1;
        if !is_readable_dword(slot) {
            break;
        }
        let entry = *(slot as *const u32);
        if entry < 0x10000 || entry & 3 != 0 || !is_readable_dword(entry) {
            invalid_run += 1;
            continue;
        }
        let vtable = *(entry as *const u32);
        if !(0x0066A000..0x00692000).contains(&vtable) {
            invalid_run += 1;
            continue;
        }
        invalid_run = 0;
        objects += 1;
        match tallies.iter_mut().find(|t| t.0 == vtable) {
            Some(t) => t.1 += 1,
            None => {
                if tallies.len() < 32 {
                    tallies.push((vtable, 1));
                }
            }
        }
    }
    tallies.sort_by_key(|t| u32::MAX - t.1);
    let list: Vec<String> = tallies.iter().map(|(v, n)| format!("{v:#010x} x{n}")).collect();
    info!("scene gfx objects: {objects} in {} slots scanned | vtables: {}", index, list.join(" | "));
}

unsafe fn is_readable_dword(address: u32) -> bool {
    let mut info: MEMORY_BASIC_INFORMATION = mem::zeroed();
    if VirtualQuery(Some(address as _), &mut info, mem::size_of::<MEMORY_BASIC_INFORMATION>()) == 0 {
        return false;
    }
    info.State == MEM_COMMIT
}

unsafe extern "C" fn on_perf_frame() {
    let now_ms = *(0x006DCCF0 as *const u32);
    let delta = *(0x006DCCF4 as *const u32);
    let tsc = core::arch::x86::_rdtsc();
    let mut p = PERF.lock().unwrap();
    if PERF_PROBE_ON.load(Ordering::Relaxed) {
        track_def_handles(&mut p);
    }
    p.frames += 1;
    p.delta_max = p.delta_max.max(delta);
    if p.bucket_start_ms == 0 {
        p.bucket_start_ms = now_ms;
        p.bucket_start_tsc = tsc;
        return;
    }
    let elapsed = now_ms.wrapping_sub(p.bucket_start_ms);
    if elapsed < 1000 {
        return;
    }
    if PERF_PROBE_ON.load(Ordering::Relaxed) {
        let cycles_per_ms = ((tsc - p.bucket_start_tsc) / elapsed as u64).max(1);
        let frames = p.frames.max(1);
        perf_log!(
            "perf: {} frames in {elapsed} ms (worst {} ms) | cache {}/{} KB | submits {} ({:.1}/frame, {} kpx/frame) | select {:.2} ms ({}x) blit {:.2} ms ({}x) | aim convert {:.2} ms ({}x, {} KB) | shipyard update {:.2} ms ({}x) draw {:.2} ms ({}x)",
            p.frames,
            p.delta_max,
            ddraw_dword(CACHE_USAGE_RVA) / 1024,
            ddraw_dword(CACHE_LIMIT_RVA) / 1024,
            p.submits,
            p.submits as f64 / frames as f64,
            p.area / frames as u64 / 1000,
            p.select_tsc as f64 / cycles_per_ms as f64,
            p.select_calls,
            p.blit_tsc as f64 / cycles_per_ms as f64,
            p.blit_calls,
            p.convert_tsc as f64 / cycles_per_ms as f64,
            p.convert_calls,
            p.convert_bytes / 1024,
            p.sy_update_tsc as f64 / cycles_per_ms as f64,
            p.sy_update_calls,
            p.sy_draw_tsc as f64 / cycles_per_ms as f64,
            p.sy_draw_calls,
        );
        p.sites.sort_by_key(|site| u64::MAX - site.2);
        for (ret, calls, area) in p.sites.iter().take(6) {
            perf_log!("perf submit site {ret:#010x}: {calls} calls, {} kpx", area / 1000);
        }
        if p.handle_changes > 0 {
            p.changed_defs.sort_by_key(|entry| u32::MAX - entry.1);
            let list: Vec<String> = p.changed_defs.iter().take(8).map(|(def, count)| format!("{def:#010x} x{count}")).collect();
            debug!(
                "perf def handle changes: {} across {} frames | {}",
                p.handle_changes, p.handle_change_frames, list.join(" | ")
            );
        }
        if p.dtor_calls > 0 {
            p.dtor_sites.sort_by_key(|entry| u32::MAX - entry.1);
            let list: Vec<String> = p.dtor_sites.iter().take(8).map(|(site, count)| format!("{site:#010x} x{count}")).collect();
            perf_log!("perf record dtors: {} | {}", p.dtor_calls, list.join(" | "));
        }
        if p.free_calls > 0 {
            p.free_sites.sort_by_key(|entry| u32::MAX - entry.1);
            let list: Vec<String> = p.free_sites.iter().take(8).map(|(site, count)| format!("{site:#010x} x{count}")).collect();
            perf_log!("perf texture frees: {} | {}", p.free_calls, list.join(" | "));
        }
        if !p.decode_dll_sites.is_empty() {
            p.decode_dll_sites.sort_by_key(|entry| u32::MAX - entry.1);
            let list: Vec<String> = p.decode_dll_sites.iter().take(8).map(|(site, count)| format!("ddraw_dll+{site:#x} x{count}")).collect();
            perf_log!("perf decode dll sites: {}", list.join(" | "));
        }
        if !p.decode_sites.is_empty() {
            p.decode_sites.sort_by_key(|entry| u32::MAX - entry.1);
            let list: Vec<String> = p.decode_sites.iter().take(8).map(|(site, count)| format!("{site:#010x} x{count}")).collect();
            perf_log!("perf decode trigger sites: {}", list.join(" | "));
        }
        if p.variant_switches > 0 {
            p.variant_records.sort_by_key(|entry| u32::MAX - entry.1);
            let list: Vec<String> = p.variant_records.iter().take(6).map(|(record, count)| format!("{record:#010x} x{count}")).collect();
            perf_log!("perf variant switches: {} | {}", p.variant_switches, list.join(" | "));
        }
        if !p.decodes.is_empty() {
            p.decodes.sort_by_key(|entry| u64::MAX - entry.2);
            let cycles_per_ms = ((tsc - p.bucket_start_tsc) / elapsed.max(1) as u64).max(1);
            let list: Vec<String> = p
                .decodes
                .iter()
                .take(8)
                .map(|(name, calls, spent)| format!("{name} x{calls} ({:.1} ms)", *spent as f64 / cycles_per_ms as f64))
                .collect();
            perf_log!("perf aim file decodes: {}", list.join(" | "));
        }
        if !p.converts.is_empty() {
            p.converts.sort_by_key(|entry| u32::MAX - entry.1);
            let list: Vec<String> = p.converts.iter().map(|(key, count)| format!("{}x{} x{count}", key >> 16, key & 0xFFFF)).collect();
            perf_log!("perf aim converts by asset: {}", list.join(" | "));
        }
    }
    p.frames = 0;
    p.delta_max = 0;
    p.submits = 0;
    p.area = 0;
    p.sites.clear();
    p.sy_update_tsc = 0;
    p.sy_update_calls = 0;
    p.sy_draw_tsc = 0;
    p.sy_draw_calls = 0;
    p.select_tsc = 0;
    p.select_calls = 0;
    p.blit_tsc = 0;
    p.blit_calls = 0;
    p.convert_tsc = 0;
    p.convert_calls = 0;
    p.convert_bytes = 0;
    p.converts.clear();
    p.decodes.clear();
    p.variant_switches = 0;
    p.variant_records.clear();
    p.decode_sites.clear();
    p.free_calls = 0;
    p.free_sites.clear();
    p.dtor_calls = 0;
    p.dtor_sites.clear();
    p.handle_changes = 0;
    p.handle_change_frames = 0;
    p.changed_defs.clear();
    p.decode_dll_sites.clear();
    p.bucket_start_ms = now_ms;
    p.bucket_start_tsc = tsc;
}

unsafe extern "C" fn on_perf_submit(ret: u32, rect: *const i32) {
    let area = ((*rect.add(2) - *rect).max(0) as u64) * ((*rect.add(3) - *rect.add(1)).max(0) as u64);
    let mut p = PERF.lock().unwrap();
    p.submits += 1;
    p.area += area;
    if let Some(site) = p.sites.iter_mut().find(|site| site.0 == ret) {
        site.1 += 1;
        site.2 += area;
    } else if p.sites.len() < 64 {
        p.sites.push((ret, 1, area));
    }
}

unsafe extern "thiscall" fn shipyard_update_perf_hook(this: u32) {
    let t0 = core::arch::x86::_rdtsc();
    let orig: extern "thiscall" fn(u32) = mem::transmute((*SY_UPDATE_HOOK_PTR.load(Ordering::SeqCst)).old_absolute);
    orig(this);
    let spent = core::arch::x86::_rdtsc() - t0;
    let mut p = PERF.lock().unwrap();
    p.sy_update_tsc += spent;
    p.sy_update_calls += 1;
}

unsafe extern "thiscall" fn shipyard_draw_perf_hook(this: u32, a: i32, b: i32, c: i32, d: i32) {
    let t0 = core::arch::x86::_rdtsc();
    let orig: extern "thiscall" fn(u32, i32, i32, i32, i32) = mem::transmute((*SY_DRAW_HOOK_PTR.load(Ordering::SeqCst)).old_absolute);
    orig(this, a, b, c, d);
    let spent = core::arch::x86::_rdtsc() - t0;
    let mut p = PERF.lock().unwrap();
    p.sy_draw_tsc += spent;
    p.sy_draw_calls += 1;
}

unsafe extern "C" fn ddraw_select_perf_hook(arg: u32) -> u32 {
    let t0 = core::arch::x86::_rdtsc();
    let orig: extern "C" fn(u32) -> u32 = mem::transmute((*DDRAW_SELECT_HOOK_PTR.load(Ordering::Relaxed)).old_absolute);
    let result = orig(arg);
    let spent = core::arch::x86::_rdtsc() - t0;
    let mut p = PERF.lock().unwrap();
    p.select_tsc += spent;
    p.select_calls += 1;
    result
}

unsafe extern "C" fn ddraw_blit_perf_hook(arg: u32) -> u32 {
    let t0 = core::arch::x86::_rdtsc();
    let orig: extern "C" fn(u32) -> u32 = mem::transmute((*DDRAW_BLIT_HOOK_PTR.load(Ordering::Relaxed)).old_absolute);
    let result = orig(arg);
    let spent = core::arch::x86::_rdtsc() - t0;
    let mut p = PERF.lock().unwrap();
    p.blit_tsc += spent;
    p.blit_calls += 1;
    result
}

/// The perf probe's sampling profiler. A watchdog thread that, while the probe is on,
/// suspends the game's main thread every ~4 ms, reads EIP, and logs the hottest
/// 256-byte code regions once a second - attribution without guessing which function
/// to wrap. Nothing is called between suspend and resume except GetThreadContext, so
/// no lock the suspended thread might hold is ever taken.
static MAIN_THREAD_ID: AtomicU32 = AtomicU32::new(0);

fn start_perf_sampler() {
    let main_tid = MAIN_THREAD_ID.load(Ordering::SeqCst);
    std::thread::spawn(move || unsafe {
        let handle = match OpenThread(THREAD_SUSPEND_RESUME | THREAD_GET_CONTEXT, false, main_tid) {
            Ok(handle) => handle,
            Err(e) => {
                error!("perf sampler: OpenThread failed: {e}");
                return;
            }
        };
        let mut buckets: Vec<(u32, u32)> = Vec::new();
        let mut samples: u32 = 0;
        let mut last_log = std::time::Instant::now();
        loop {
            if !PERF_PROBE_ON.load(Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(200));
                buckets.clear();
                samples = 0;
                last_log = std::time::Instant::now();
                continue;
            }
            std::thread::sleep(std::time::Duration::from_millis(4));
            if SuspendThread(handle) == u32::MAX {
                continue;
            }
            let mut ctx: CONTEXT = mem::zeroed();
            ctx.ContextFlags = 0x00010001; // CONTEXT_i386 | CONTEXT_CONTROL
            let ok = GetThreadContext(handle, &mut ctx);
            ResumeThread(handle);
            if !ok.as_bool() {
                continue;
            }
            samples += 1;
            let bucket = ctx.Eip >> 8;
            match buckets.iter_mut().find(|b| b.0 == bucket) {
                Some(b) => b.1 += 1,
                None => buckets.push((bucket, 1)),
            }
            if last_log.elapsed() >= std::time::Duration::from_secs(1) {
                buckets.sort_by_key(|b| u32::MAX - b.1);
                let total = samples.max(1);
                let top: Vec<String> = buckets
                    .iter()
                    .take(10)
                    .map(|(bucket, count)| format!("{:#010x} {}% [{}]", bucket << 8, count * 100 / total, resolve_module(bucket << 8)))
                    .collect();
                perf_log!("perf sample ({samples}x): {}", top.join(" | "));
                buckets.clear();
                samples = 0;
                last_log = std::time::Instant::now();
            }
        }
    });
}

/// Name the module containing `address` as "file+rva", or "private" for memory outside
/// any loaded module (VirtualAlloc'd code, JIT regions).
unsafe fn resolve_module(address: u32) -> String {
    let mut module = windows::Win32::Foundation::HMODULE::default();
    if GetModuleHandleExA(
        GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
        windows::core::PCSTR::from_raw(address as _),
        &mut module,
    )
    .as_bool()
    {
        let mut buffer = [0u8; 260];
        let len = GetModuleFileNameA(module, &mut buffer) as usize;
        if len > 0 {
            let path = String::from_utf8_lossy(&buffer[..len]).into_owned();
            let file = path.rsplit('\\').next().unwrap_or(&path).to_string();
            return format!("{file}+{:#x}", address.wrapping_sub(module.0 as u32));
        }
    }
    "private".to_string()
}

unsafe extern "C" fn aim_convert_perf_hook(image: *mut u8, ext: *const u8, data: *const u8, size: i32) -> i32 {
    let t0 = core::arch::x86::_rdtsc();
    let orig: extern "C" fn(*mut u8, *const u8, *const u8, i32) -> i32 = mem::transmute((*AIM_CONVERT_HOOK_PTR.load(Ordering::Relaxed)).old_absolute);
    let result = orig(image, ext, data, size);
    let spent = core::arch::x86::_rdtsc() - t0;
    let mut p = PERF.lock().unwrap();
    p.convert_tsc += spent;
    p.convert_calls += 1;
    p.convert_bytes += size.max(0) as u64;
    let size = size.max(0) as u32;
    if let Some(entry) = p.converts.iter_mut().find(|entry| entry.0 == size) {
        entry.1 += 1;
    } else if p.converts.len() < 32 {
        p.converts.push((size, 1));
    }
    result
}

/// AIM_CONVERT_RAW takes only the AIM_IMAGE; identify the asset by the decoded
/// dimensions afterwards (struct head = [pixels, palette?, width, height, ...]),
/// tallied into the same per-asset list as key (width << 16 | height).
unsafe extern "C" fn aim_convert_raw_perf_hook(image: *mut u8) -> i32 {
    let t0 = core::arch::x86::_rdtsc();
    let orig: extern "C" fn(*mut u8) -> i32 = mem::transmute((*AIM_CONVERT_RAW_HOOK_PTR.load(Ordering::Relaxed)).old_absolute);
    let result = orig(image);
    let spent = core::arch::x86::_rdtsc() - t0;
    let width = *(image.add(8) as *const u32);
    let height = *(image.add(12) as *const u32);
    let key = (width << 16) | (height & 0xFFFF);
    let mut p = PERF.lock().unwrap();
    p.convert_tsc += spent;
    p.convert_calls += 1;
    if let Some(entry) = p.converts.iter_mut().find(|entry| entry.0 == key) {
        entry.1 += 1;
    } else if p.converts.len() < 32 {
        p.converts.push((key, 1));
    }
    result
}

unsafe extern "cdecl" fn aim_decode_file_perf_hook(image_inner: u32, file_path: *const u8, file_data: u32, file_size: u32) -> i32 {
    // Attribute the decode to the exe code that triggered it: scan the stack upward
    // from our own frame for the first dword inside the exe's .text (0x401000..0x66A000).
    // The stack holds aim.dll and ddraw_dll return addresses below it; the first exe
    // address is (approximately) the sgl call site that caused the load.
    let mut trigger = 0u32;
    let mut dll_sites = [0u32; 3];
    let mut dll_found = 0usize;
    let dll_base = ddraw_base();
    let anchor = (&file_path) as *const _ as u32;
    let mut cursor = anchor & !3;
    let ceiling = cursor + 0x400;
    while cursor < ceiling {
        let value = *(cursor as *const u32);
        if trigger == 0 && EXE_CODE.contains(&value) {
            trigger = value;
        }
        let dll_rva = value.wrapping_sub(dll_base);
        if dll_found < 3 && dll_base != 0 && DDRAW_CODE_RVAS.contains(&dll_rva) && !dll_sites.contains(&dll_rva) {
            dll_sites[dll_found] = dll_rva;
            dll_found += 1;
        }
        if trigger != 0 && dll_found == 3 {
            break;
        }
        cursor += 4;
    }
    let t0 = core::arch::x86::_rdtsc();
    let orig: extern "cdecl" fn(u32, *const u8, u32, u32) -> i32 = mem::transmute((*AIM_DECODE_FILE_HOOK_PTR.load(Ordering::Relaxed)).old_absolute);
    let result = orig(image_inner, file_path, file_data, file_size);
    let spent = core::arch::x86::_rdtsc() - t0;
    // File name tail only (paths are latin1; ASCII basenames in practice).
    let mut name = Vec::new();
    let mut cursor = file_path;
    while !cursor.is_null() && *cursor != 0 && name.len() < 256 {
        name.push(*cursor);
        cursor = cursor.add(1);
    }
    let name = String::from_utf8_lossy(&name).replace('\\', "/");
    let name = name.rsplit('/').next().unwrap_or(&name).to_string();
    let mut p = PERF.lock().unwrap();
    if let Some(entry) = p.decodes.iter_mut().find(|entry| entry.0 == name) {
        entry.1 += 1;
        entry.2 += spent;
    } else if p.decodes.len() < 64 {
        p.decodes.push((name, 1, spent));
    }
    if trigger != 0 {
        if let Some(entry) = p.decode_sites.iter_mut().find(|entry| entry.0 == trigger) {
            entry.1 += 1;
        } else if p.decode_sites.len() < 32 {
            p.decode_sites.push((trigger, 1));
        }
    }
    for &dll_site in dll_sites.iter().take(dll_found) {
        if let Some(entry) = p.decode_dll_sites.iter_mut().find(|entry| entry.0 == dll_site) {
            entry.1 += 1;
        } else if p.decode_dll_sites.len() < 32 {
            p.decode_dll_sites.push((dll_site, 1));
        }
    }
    result
}

unsafe extern "C" fn on_perf_variant_switch(record: u32, new_variant: u32) {
    let current = *((record + 0x38) as *const u8) as u32;
    if current == (new_variant & 0xFF) {
        return;
    }
    let mut p = PERF.lock().unwrap();
    p.variant_switches += 1;
    if let Some(entry) = p.variant_records.iter_mut().find(|entry| entry.0 == record) {
        entry.1 += 1;
    } else if p.variant_records.len() < 32 {
        p.variant_records.push((record, 1));
    }
}

unsafe extern "C" fn free_memory_texture_perf_hook(texture: u32) -> u32 {
    // The thunk jmp does not push, so the first exe .text address above our frame is
    // the direct caller of sgl_FreeMemoryTexture.
    let mut site = 0u32;
    let anchor = (&texture) as *const _ as u32;
    let mut cursor = anchor & !3;
    let ceiling = cursor + 0x100;
    while cursor < ceiling {
        let value = *(cursor as *const u32);
        if EXE_CODE.contains(&value) {
            site = value;
            break;
        }
        cursor += 4;
    }
    {
        let mut p = PERF.lock().unwrap();
        p.free_calls += 1;
        if site != 0 {
            if let Some(entry) = p.free_sites.iter_mut().find(|entry| entry.0 == site) {
                entry.1 += 1;
            } else if p.free_sites.len() < 32 {
                p.free_sites.push((site, 1));
            }
        }
    }
    let orig: extern "C" fn(u32) -> u32 = mem::transmute((*FREE_MEMORY_TEXTURE_HOOK_PTR.load(Ordering::Relaxed)).old_absolute);
    orig(texture)
}

unsafe extern "C" fn on_perf_record_dtor(record: u32) {
    let _ = record;
    // Attribute to the first exe .text return address OUTSIDE the record class's own
    // code (0x4B2000..0x4B5000) - the virtual-call site that destroys the record.
    let mut site = 0u32;
    let anchor = (&record) as *const _ as u32;
    let mut cursor = anchor & !3;
    let ceiling = cursor + 0x200;
    while cursor < ceiling {
        let value = *(cursor as *const u32);
        if EXE_CODE.contains(&value) && !(0x004B2000..0x004B5000).contains(&value) {
            site = value;
            break;
        }
        cursor += 4;
    }
    let mut p = PERF.lock().unwrap();
    p.dtor_calls += 1;
    if site != 0 {
        if let Some(entry) = p.dtor_sites.iter_mut().find(|entry| entry.0 == site) {
            entry.1 += 1;
        } else if p.dtor_sites.len() < 32 {
            p.dtor_sites.push((site, 1));
        }
    }
}


/// Snapshot every graphic definition's texture handle (def+0x34) and count changes
/// against the previous frame. The def index is a byte, so the array holds at most
/// 256 entries - cheap enough per frame. Def pointers are validated once each.
unsafe fn track_def_handles(p: &mut PerfState) {
    let count = *(0x006E2E6C as *const u32);
    let table = *(0x006E2E70 as *const u32);
    if table == 0 || count == 0 || count > 0x400 || !is_readable_dword(table) {
        return;
    }
    let mut current: Vec<(u32, u32)> = Vec::with_capacity(count as usize);
    for index in 0..count {
        let def = *((table + index * 4) as *const u32);
        if def < 0x10000 || def & 3 != 0 {
            continue;
        }
        if !p.validated_defs.contains(&def) {
            if !is_readable_dword(def + 0x34) {
                continue;
            }
            if p.validated_defs.len() < 1024 {
                p.validated_defs.push(def);
            }
        }
        current.push((def, *((def + 0x34) as *const u32)));
    }
    let mut changes_this_frame = 0u32;
    for &(def, handle) in &current {
        if let Some(&(_, previous)) = p.prev_handles.iter().find(|(d, _)| *d == def) {
            if previous != handle {
                changes_this_frame += 1;
                if let Some(entry) = p.changed_defs.iter_mut().find(|entry| entry.0 == def) {
                    entry.1 += 1;
                } else if p.changed_defs.len() < 64 {
                    p.changed_defs.push((def, 1));
                }
            }
        }
    }
    if changes_this_frame > 0 {
        p.handle_changes += changes_this_frame;
        p.handle_change_frames += 1;
    }
    p.prev_handles = current;
}

/// The budget in force before [`toggle_cache_budget`] raised it; 0 while not overridden.
static SAVED_CACHE_LIMIT: AtomicU32 = AtomicU32::new(0);

/// A/B the decoded-image cache budget at runtime: toggle between whatever is in force
/// (the library's 16 MiB default, or the value this mod patched in) and 512 MB. The LRU
/// evictors at `ddraw_dll+0x1C2EF`/`+0x1C39A` release images while the usage counter
/// exceeds the budget, so a budget below the working set evicts exactly what the next
/// frame redraws - the re-decode storm this profiler was written to measure.
pub unsafe fn toggle_cache_budget() {
    let base = ddraw_base();
    if base == 0 {
        error!("texture cache budget: ddraw_dll.dll is not loaded");
        return;
    }
    let limit = (base + CACHE_LIMIT_RVA) as *mut u32;
    if SAVED_CACHE_LIMIT.load(Ordering::SeqCst) == 0 {
        SAVED_CACHE_LIMIT.store(*limit, Ordering::SeqCst);
        *limit = 0x2000_0000;
        info!("texture cache budget: 512 MB (was {:#010x})", SAVED_CACHE_LIMIT.load(Ordering::SeqCst));
    } else {
        *limit = SAVED_CACHE_LIMIT.load(Ordering::SeqCst);
        SAVED_CACHE_LIMIT.store(0, Ordering::SeqCst);
        info!("texture cache budget: restored ({:#010x})", *limit);
    }
}

/// Forces translucent-white constant colors to opaque white while no-dim is on -
/// the modulated blit path then short-circuits to plain surface blits, skipping the
/// per-frame transient re-decode of source images.
unsafe extern "C" fn set_constant_color_hook(color: u32) -> u32 {
    let mut color = color;
    if NO_DIM.load(Ordering::Relaxed) && (color >> 24) != 0xFF && (color & 0x00FF_FFFF) == 0x00FF_FFFF {
        color = 0xFFFF_FFFF;
    }
    let orig: extern "C" fn(u32) -> u32 = mem::transmute((*SET_CONSTANT_COLOR_HOOK_PTR.load(Ordering::Relaxed)).old_absolute);
    orig(color)
}
