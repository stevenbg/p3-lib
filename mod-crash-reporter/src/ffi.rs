//! Crash reporter: writes a full report to `_crash_report.txt` in the game
//! directory whenever the game raises a fatal-severity exception.
//!
//! The game dies to desktop without a dump, a WER dialog or DebugView output, so
//! crashes are captured first-chance: a vectored exception handler registered at
//! the front of the chain sees every exception before any of the game's SEH
//! handlers run, and appends a report - registers, the code bytes at EIP, the
//! memory around each register value, the SEH chain, the EBP call chain, a scan of
//! the stack for return addresses and the loaded module map (all attributed as
//! module+offset) - before anything can swallow the exception or kill the process.
//! Only fatal-severity codes are reported (C++ throws and debug prints are
//! routine); a report the process survives was a handled exception, so the last
//! report in the file is the crash. The unhandled-exception filter marks the report
//! that killed the process, and repeats it in full only when it is a different
//! fault.
//!
//! Known blind spot: heap-corruption kills via fail-fast (`int 29`) never enter
//! exception dispatch, so a crash that leaves no report here points at the heap
//! (next step: PageHeap via `gflags /p /enable Patrician3.exe /full`, which turns
//! the corruption into an access violation this reporter catches).
//!
//! This mod found the patrol-letter crash fixed by mod-fix-patrol-letter-crash.
//! The dump was then widened by what the d3d9 lost-device crash needed and did not
//! get (`.claude/notes/todo/device-lost-crash.md`): the bad pointer lived at
//! `object+0x78`, just outside the 0x40 register window of the time; the stack scan
//! listed SEH handlers and stale dwords indistinguishably from return addresses;
//! and nothing said which of the two same-named `ddraw` DLLs a `module+offset`
//! belonged to, or that the faulting address was in the never-mapped first 64 KB.

use log::info;
use std::fmt::Write as _;
use std::io::Write as _;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use windows::Win32::{
    Foundation::HMODULE,
    System::{
        Diagnostics::Debug::{AddVectoredExceptionHandler, SetUnhandledExceptionFilter, CONTEXT, EXCEPTION_POINTERS, EXCEPTION_RECORD},
        LibraryLoader::GetModuleFileNameA,
        Memory::{
            VirtualQuery, MEMORY_BASIC_INFORMATION, MEM_COMMIT, MEM_FREE, MEM_IMAGE, MEM_RESERVE, PAGE_EXECUTE, PAGE_EXECUTE_READ,
            PAGE_EXECUTE_READWRITE, PAGE_EXECUTE_WRITECOPY, PAGE_GUARD, PAGE_NOACCESS,
        },
        Threading::GetCurrentThreadId,
    },
};

/// Reports are appended here, relative to the game's working directory (next to
/// Patrician3.exe, like the save folder).
const REPORT_PATH: &str = "_crash_report.txt";
/// The MSVC C++ throw code: raised and caught routinely, not a crash.
const MSVC_CPP_EXCEPTION: u32 = 0xE06D7363;
const EXCEPTION_CONTINUE_SEARCH: i32 = 0;

/// Bytes dumped around each register value, and how much of that sits below it.
/// 0x100 covers the object layouts this game and its graphics stack use: the d3d9
/// resource whose corrupt linked-list pointer at `+0x78` caused the lost-device
/// crash fell outside the original 0x40 window, so the report could not say where
/// the bad pointer had come from.
const REGISTER_WINDOW: u32 = 0x100;
const REGISTER_WINDOW_BELOW: u32 = 0x20;

static IN_HANDLER: AtomicBool = AtomicBool::new(false);
static REPORT_COUNT: AtomicU32 = AtomicU32::new(0);
/// The last report written, so the unhandled filter can mark it rather than append a
/// second identical copy of the same fault.
static LAST_INDEX: AtomicU32 = AtomicU32::new(u32::MAX);
static LAST_CODE: AtomicU32 = AtomicU32::new(0);
static LAST_ADDRESS: AtomicU32 = AtomicU32::new(0);
static LAST_ESP: AtomicU32 = AtomicU32::new(0);

#[derive(PartialEq)]
enum Kind {
    FirstChance,
    Unhandled,
}

#[no_mangle]
pub unsafe extern "C" fn start() -> u32 {
    let _ = log::set_logger(&win_dbg_logger::DEBUGGER_LOGGER);
    log::set_max_level(log::LevelFilter::Trace);

    // First-position VEH: runs before every frame-based handler, so the report is
    // written even if something swallows the exception or the process dies without
    // reaching the unhandled filter.
    AddVectoredExceptionHandler(1, Some(vectored_handler));
    SetUnhandledExceptionFilter(Some(unhandled_filter));
    info!("installed");
    0
}

unsafe extern "system" fn vectored_handler(info: *mut EXCEPTION_POINTERS) -> i32 {
    report(info as *const EXCEPTION_POINTERS, Kind::FirstChance);
    EXCEPTION_CONTINUE_SEARCH
}

unsafe extern "system" fn unhandled_filter(info: *const EXCEPTION_POINTERS) -> i32 {
    report(info, Kind::Unhandled);
    EXCEPTION_CONTINUE_SEARCH
}

unsafe fn report(info: *const EXCEPTION_POINTERS, kind: Kind) {
    let Some(info) = info.as_ref() else { return };
    let Some(record) = info.ExceptionRecord.as_ref() else { return };
    let code = record.ExceptionCode.0 as u32;
    // Fatal severity only (top two bits set): skips C++ throws, DBG_PRINTEXCEPTION
    // from OutputDebugString, breakpoints and other routine dispatch traffic.
    if code >> 30 != 3 || code == MSVC_CPP_EXCEPTION {
        return;
    }
    if IN_HANDLER.swap(true, Ordering::SeqCst) {
        // An exception inside the handler itself (or a concurrent one): bail rather
        // than recurse.
        return;
    }

    let address = record.ExceptionAddress as u32;
    let esp = info.ContextRecord.as_ref().map_or(0, |ctx| ctx.Esp);
    let last = LAST_INDEX.load(Ordering::SeqCst);
    // The unhandled filter sees the fault the first-chance handler already reported.
    // Same code, same faulting address, same stack pointer means the same fault, so
    // mark that report fatal instead of duplicating it.
    let repeat = kind == Kind::Unhandled
        && last != u32::MAX
        && LAST_CODE.load(Ordering::SeqCst) == code
        && LAST_ADDRESS.load(Ordering::SeqCst) == address
        && LAST_ESP.load(Ordering::SeqCst) == esp;

    if repeat {
        append(&format!("=== report {last} was UNHANDLED - the process went down on it ===\n"));
    } else {
        LAST_CODE.store(code, Ordering::SeqCst);
        LAST_ADDRESS.store(address, Ordering::SeqCst);
        LAST_ESP.store(esp, Ordering::SeqCst);
        let label = match kind {
            Kind::FirstChance => "first-chance",
            Kind::Unhandled => "UNHANDLED - process is going down",
        };
        write_report(info, record, code, label);
    }
    IN_HANDLER.store(false, Ordering::SeqCst);
}

unsafe fn write_report(info: &EXCEPTION_POINTERS, record: &EXCEPTION_RECORD, code: u32, kind: &str) {
    let n = REPORT_COUNT.fetch_add(1, Ordering::SeqCst);
    LAST_INDEX.store(n, Ordering::SeqCst);
    let unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let mut out = String::with_capacity(32768);
    let _ = writeln!(
        out,
        "=== report {n}: {kind} {code:#010x} {} [unix {unix}, thread {:#x}] ===",
        code_name(code),
        GetCurrentThreadId()
    );
    let _ = writeln!(out, "at {}", describe_address(record.ExceptionAddress as u32));
    if code == 0xC0000005 && record.NumberParameters >= 2 {
        let op = match record.ExceptionInformation[0] {
            0 => "read",
            1 => "write",
            8 => "execute",
            _ => "access",
        };
        let target = record.ExceptionInformation[1] as u32;
        let _ = writeln!(out, "{op} of address {target:#010x} - {}", describe_target(target));
    }

    if let Some(ctx) = info.ContextRecord.as_ref() {
        let _ = writeln!(
            out,
            "eax={:08x} ebx={:08x} ecx={:08x} edx={:08x} esi={:08x} edi={:08x}",
            ctx.Eax, ctx.Ebx, ctx.Ecx, ctx.Edx, ctx.Esi, ctx.Edi
        );
        let _ = writeln!(
            out,
            "eip={:08x} ebp={:08x} esp={:08x} eflags={:08x}",
            ctx.Eip, ctx.Ebp, ctx.Esp, ctx.EFlags
        );
        dump_code(&mut out, ctx.Eip);
        dump_register_memory(&mut out, ctx);
        let handlers = dump_seh_chain(&mut out);
        walk_ebp_chain(&mut out, ctx);
        scan_stack(&mut out, ctx.Esp, &handlers);
    } else {
        let _ = writeln!(out, "(no context record)");
    }
    dump_modules(&mut out);
    let _ = writeln!(out, "=== end report {n} ===");

    append(&out);
}

fn append(text: &str) {
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(REPORT_PATH) {
        let _ = f.write_all(text.as_bytes());
        let _ = f.flush();
    }
}

/// The code bytes around EIP, so the faulting instruction is identifiable even if the
/// address is not in a module (jumps through corrupted pointers land anywhere).
unsafe fn dump_code(out: &mut String, eip: u32) {
    let _ = writeln!(out, "code around eip:");
    let start = eip.saturating_sub(0x20) & !0xF;
    for row in 0..6u32 {
        let addr = start + row * 16;
        // Rows are 16-aligned and regions page-aligned, so one check covers the row.
        if !is_readable(addr) {
            let _ = writeln!(out, "  {addr:08x}: ??");
            continue;
        }
        let _ = write!(out, "  {addr:08x}:");
        for i in 0..16 {
            let b = core::ptr::read_unaligned((addr + i) as *const u8);
            let marker = if addr + i == eip { '>' } else { ' ' };
            let _ = write!(out, "{marker}{b:02x}");
        }
        let _ = writeln!(out);
    }
}

/// `REGISTER_WINDOW` bytes around each register that points into readable memory: the
/// crash-time contents of whatever structures the faulting code was working on. The
/// window starts below the register so that a heap block's header is visible - the
/// eight bytes before an allocation say whether the block is live and how big it is,
/// which is how a use-after-free is told apart from plain corruption.
///
/// Unreadable rows are reported rather than skipping the whole register, so a pointer
/// near the end of its region still yields the part that is there.
unsafe fn dump_register_memory(out: &mut String, ctx: &CONTEXT) {
    let regs = [
        ("eax", ctx.Eax),
        ("ebx", ctx.Ebx),
        ("ecx", ctx.Ecx),
        ("edx", ctx.Edx),
        ("esi", ctx.Esi),
        ("edi", ctx.Edi),
        ("ebp", ctx.Ebp),
        ("esp", ctx.Esp),
    ];
    let mut dumped: Vec<(u32, &str)> = Vec::new();
    for (name, value) in regs {
        if !is_readable(value) {
            continue;
        }
        let start = (value & !0xf).saturating_sub(REGISTER_WINDOW_BELOW);
        if let Some((_, first)) = dumped.iter().find(|(s, _)| *s == start) {
            let _ = writeln!(out, "memory at {name} ({value:#010x}): same window as {first}");
            continue;
        }
        dumped.push((start, name));
        let _ = writeln!(out, "memory at {name} ({value:#010x}):");
        for row in 0..REGISTER_WINDOW / 16 {
            let addr = start + row * 16;
            if !is_readable(addr) {
                let _ = writeln!(out, "  {addr:08x}: ??");
                continue;
            }
            let bytes: Vec<String> = (0..16).map(|i| format!("{:02x}", core::ptr::read((addr + i) as *const u8))).collect();
            let _ = writeln!(out, "  {addr:08x}: {}", bytes.join(" "));
        }
    }
}

/// The thread's SEH chain from `fs:[0]`. Worth its own section for two reasons: the
/// innermost handler names the function that was expecting to catch something, and the
/// handler addresses are on the stack, where the stack scan below would otherwise
/// report them as return-address candidates. Returns the handlers so the scan can
/// label them.
unsafe fn dump_seh_chain(out: &mut String) -> Vec<u32> {
    let _ = writeln!(out, "seh chain:");
    let mut handlers = Vec::new();
    let mut record: u32;
    core::arch::asm!("mov {}, fs:[0]", out(reg) record);
    for i in 0..32 {
        // The chain ends at -1; a record is { prev, handler }.
        if record == u32::MAX {
            break;
        }
        let (Some(prev), Some(handler)) = (read_u32(record), read_u32(record + 4)) else {
            let _ = writeln!(out, "  #{i} record={record:08x} (unreadable)");
            break;
        };
        let _ = writeln!(out, "  #{i} record={record:08x} handler={}", describe_address(handler));
        handlers.push(handler);
        // Records live on the stack and must ascend, or the chain is garbage.
        if prev <= record {
            break;
        }
        record = prev;
    }
    handlers
}

/// The classic x86 frame walk. MSVC-era code keeps frame pointers in most functions;
/// where a frame omits them the chain just ends early and the stack scan takes over.
unsafe fn walk_ebp_chain(out: &mut String, ctx: &CONTEXT) {
    let _ = writeln!(out, "ebp chain:");
    let mut ebp = ctx.Ebp;
    for i in 0..32 {
        let (Some(next), Some(ret)) = (read_u32(ebp), read_u32(ebp + 4)) else {
            let _ = writeln!(out, "  #{i} ebp={ebp:08x} (unreadable)");
            break;
        };
        let _ = writeln!(out, "  #{i} ebp={ebp:08x} ret={}", describe_address(ret));
        // Frames must ascend and stay plausibly close, or the chain is garbage.
        if ret == 0 || next <= ebp || next - ebp > 0x0010_0000 {
            break;
        }
        ebp = next;
    }
}

/// Every dword on the stack that points into module code is a return-address
/// candidate: recovers the call history even through frames without frame pointers,
/// at the price of some stale entries.
///
/// Each hit is classified, because the raw list mixes unrelated things and reading a
/// handler address as a frame sends an investigation down the wrong path. `ret` means
/// the bytes before it really do encode a call, so it is a return address - though
/// possibly a stale one, left by a call that has already returned; this test cannot
/// tell live from stale, only a return address from something that never was one.
/// `seh handler` means it came from an exception registration record. An unmarked
/// entry is neither: a function pointer, a vtable slot or plain data.
unsafe fn scan_stack(out: &mut String, esp: u32, seh_handlers: &[u32]) {
    let _ = writeln!(out, "stack scan (dwords at esp pointing into module code):");
    for off in (0..0x400u32).step_by(4) {
        let Some(value) = read_u32(esp + off) else { break };
        if !is_module_code(value) {
            continue;
        }
        let tag = if seh_handlers.contains(&value) {
            "  <- seh handler"
        } else if preceded_by_call(value) {
            "  <- ret"
        } else {
            ""
        };
        let _ = writeln!(out, "  esp+{off:#05x}: {}{tag}", describe_address(value));
    }
}

/// Whether the bytes before `addr` encode a `call` whose next instruction is exactly
/// `addr` - the test that separates real return addresses from the stale dwords and
/// SEH handler pointers a stack scan also turns up. Covers the forms MSVC emits:
/// `E8 rel32`, and `FF /2` against a register, `[reg]`, `[reg+disp8]`, `[reg+disp32]`
/// or `[disp32]`, with or without a SIB byte - each operand form fixing a different
/// instruction length, which is what has to line up with `addr`.
///
/// A heuristic: data can coincidentally end in a byte pattern that decodes as a call.
unsafe fn preceded_by_call(addr: u32) -> bool {
    // Callers only pass addresses inside a module's code, so the bytes before are in
    // the same image except right at a section start.
    if !is_readable(addr.wrapping_sub(8)) || !is_readable(addr.wrapping_sub(1)) {
        return false;
    }
    let byte_at = |back: u32| core::ptr::read(addr.wrapping_sub(back) as *const u8);

    if byte_at(5) == 0xE8 {
        return true;
    }
    for back in [2u32, 3, 4, 6, 7] {
        if byte_at(back) != 0xFF {
            continue;
        }
        let modrm = byte_at(back - 1);
        // The modrm's reg field selects the group-5 opcode; 2 is near call.
        if modrm & 0x38 != 0x10 {
            continue;
        }
        let rm = modrm & 7;
        let length = match modrm & 0xC0 {
            // register operand
            0xC0 => 2,
            // [reg], except rm 4 (SIB follows) and rm 5 (absolute disp32)
            0x00 if rm == 4 => 3,
            0x00 if rm == 5 => 6,
            0x00 => 2,
            // [reg+disp8]
            0x40 if rm == 4 => 4,
            0x40 => 3,
            // [reg+disp32]
            _ if rm == 4 => 7,
            _ => 6,
        };
        if length == back {
            return true;
        }
    }
    false
}

/// Every mapped image with its load range and full path. A report is a list of
/// `module+offset` pairs that only mean something once the exact file they came from
/// is known - this game ships two DLLs called ddraw (GOG's DirectDraw-to-D3D9 wrapper
/// and Ascaron's SGL as `ddraw_Dll.dll`) and loads the system `d3d9.dll` behind them,
/// and disassembling the wrong one wastes an evening.
unsafe fn dump_modules(out: &mut String) {
    let _ = writeln!(out, "modules:");
    let mut seen: Vec<u32> = Vec::new();
    let mut addr: u64 = 0;
    // One query per region, capped: a crash handler must not be able to spin.
    for _ in 0..4096 {
        if addr >= 0x1_0000_0000 {
            break;
        }
        let Some(mbi) = query(addr as u32) else { break };
        if mbi.State == MEM_COMMIT && mbi.Type == MEM_IMAGE {
            let base = mbi.AllocationBase as u32;
            if !seen.contains(&base) {
                seen.push(base);
                let end = image_size(base).map_or(0, |size| base.wrapping_add(size));
                let path = module_path(base).unwrap_or_else(|| "?".to_string());
                let _ = writeln!(out, "  {base:08x}-{end:08x} {path}");
            }
        }
        // RegionSize is page-granular and never zero, but do not trust it to advance.
        addr += (mbi.RegionSize as u64).max(0x1000);
    }
}

/// `SizeOfImage` from the module's own optional header, for the end of its load range.
unsafe fn image_size(base: u32) -> Option<u32> {
    if read_u32(base)? & 0xffff != 0x5A4D {
        return None; // not "MZ"
    }
    let pe = read_u32(base + 0x3c)?;
    if pe > 0x1000 || read_u32(base + pe)? != 0x0000_4550 {
        return None; // not "PE\0\0"
    }
    read_u32(base + pe + 0x50)
}

/// What the faulting address actually is - the difference between a wild pointer into
/// the never-mapped first 64 KB, a freed or decommitted page, a guard page and live
/// memory. Each points at a different kind of bug.
unsafe fn describe_target(addr: u32) -> String {
    let Some(mbi) = query(addr) else { return "address space unqueryable".to_string() };
    let base = mbi.BaseAddress as u32;
    let region = format!("region {base:#010x}..{:#010x}", base.wrapping_add(mbi.RegionSize as u32));
    if mbi.State == MEM_FREE {
        return format!("never mapped, {region}");
    }
    if mbi.State == MEM_RESERVE {
        return format!("reserved but not committed, {region}");
    }
    let what = if mbi.Type == MEM_IMAGE {
        module_of(addr).map_or_else(|| "image".to_string(), |(name, base)| format!("in {name}+{:#x}", addr - base))
    } else {
        "private".to_string()
    };
    format!("committed {} {what}, {region}", protect_name(mbi.Protect.0))
}

/// The PAGE_* values are written out rather than matched against the crate's
/// constants: the point is to name whatever a page happens to be, including the guard
/// bit combined with any of them.
fn protect_name(protect: u32) -> String {
    let base = match protect & 0xff {
        0x01 => "no-access",
        0x02 => "r--",
        0x04 => "rw-",
        0x08 => "rw- copy",
        0x10 => "--x",
        0x20 => "r-x",
        0x40 => "rwx",
        0x80 => "r-x copy",
        _ => "?",
    };
    if protect & 0x100 != 0 {
        format!("{base} guard")
    } else {
        base.to_string()
    }
}

unsafe fn describe_address(addr: u32) -> String {
    match module_of(addr) {
        Some((name, base)) => format!("{addr:#010x} ({name}+{:#x})", addr - base),
        None => format!("{addr:#010x}"),
    }
}

/// The module (mapped image) containing `addr`, as (basename, load base).
unsafe fn module_of(addr: u32) -> Option<(String, u32)> {
    let mbi = query(addr)?;
    if mbi.State != MEM_COMMIT || mbi.Type != MEM_IMAGE {
        return None;
    }
    let base = mbi.AllocationBase as u32;
    let path = module_path(base)?;
    let name = path.rsplit(['\\', '/']).next().unwrap_or(&path).to_string();
    Some((name, base))
}

unsafe fn module_path(base: u32) -> Option<String> {
    let mut buf = [0u8; 260];
    let len = GetModuleFileNameA(HMODULE(base as isize), &mut buf) as usize;
    if len == 0 || len >= buf.len() {
        return None;
    }
    Some(String::from_utf8_lossy(&buf[..len]).into_owned())
}

unsafe fn query(addr: u32) -> Option<MEMORY_BASIC_INFORMATION> {
    let mut mbi: MEMORY_BASIC_INFORMATION = core::mem::zeroed();
    if VirtualQuery(Some(addr as *const core::ffi::c_void), &mut mbi, core::mem::size_of::<MEMORY_BASIC_INFORMATION>()) == 0 {
        return None;
    }
    Some(mbi)
}

unsafe fn is_readable(addr: u32) -> bool {
    query(addr).is_some_and(|mbi| {
        mbi.State == MEM_COMMIT && mbi.Protect.0 != 0 && mbi.Protect.0 & (PAGE_NOACCESS.0 | PAGE_GUARD.0) == 0
    })
}

unsafe fn is_module_code(addr: u32) -> bool {
    query(addr).is_some_and(|mbi| {
        mbi.State == MEM_COMMIT
            && mbi.Type == MEM_IMAGE
            && mbi.Protect.0 & (PAGE_EXECUTE.0 | PAGE_EXECUTE_READ.0 | PAGE_EXECUTE_READWRITE.0 | PAGE_EXECUTE_WRITECOPY.0) != 0
    })
}

unsafe fn read_u32(addr: u32) -> Option<u32> {
    // The dword must not cross out of its (page-aligned) region.
    let mbi = query(addr)?;
    let region_end = mbi.BaseAddress as u32 + mbi.RegionSize as u32;
    if mbi.State != MEM_COMMIT || mbi.Protect.0 == 0 || mbi.Protect.0 & (PAGE_NOACCESS.0 | PAGE_GUARD.0) != 0 || addr > region_end - 4 {
        return None;
    }
    Some(core::ptr::read_unaligned(addr as *const u32))
}

fn code_name(code: u32) -> &'static str {
    match code {
        0xC0000005 => "ACCESS_VIOLATION",
        0xC000001D => "ILLEGAL_INSTRUCTION",
        0xC0000025 => "NONCONTINUABLE_EXCEPTION",
        0xC000008C => "ARRAY_BOUNDS_EXCEEDED",
        0xC000008E => "FLOAT_DIVIDE_BY_ZERO",
        0xC0000090 => "FLOAT_INVALID_OPERATION",
        0xC0000094 => "INTEGER_DIVIDE_BY_ZERO",
        0xC0000095 => "INTEGER_OVERFLOW",
        0xC0000096 => "PRIVILEGED_INSTRUCTION",
        0xC00000FD => "STACK_OVERFLOW",
        0xC0000374 => "HEAP_CORRUPTION",
        0xC0000409 => "FAIL_FAST/STACK_BUFFER_OVERRUN",
        0xC0000417 => "INVALID_CRUNTIME_PARAMETER",
        _ => "",
    }
}
