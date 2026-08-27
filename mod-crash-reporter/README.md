# mod-crash-reporter

Writes a crash report to `_crash_report.txt` in the game directory whenever the
game raises a fatal exception. Patrician 3 normally dies to desktop silently - no
minidump, no error dialog, nothing in DebugView - which makes crashes nearly
impossible to diagnose; with this mod loaded, every crash leaves a report behind.

Install: put `crash_reporter.dll` into the `mods` folder (requires the
modloader). On load it logs "installed" (visible in DebugView when the gated
mod logging is unlocked, see mod-tavern-details).

## How it works

A vectored exception handler registered at the front of the chain sees every
exception first-chance, before any of the game's own handlers run - so the report
is on disk even if something swallows the exception or kills the process
afterwards. Only fatal-severity codes are reported; C++ throws, debug prints and
breakpoints are routine dispatch traffic and are skipped. An unhandled-exception
filter additionally marks reports the process did not survive, when the game's
own filter does not preempt it.
First-chance access violations that fault inside a `C:\WINDOWS` module are not
reported: Windows' text-services stack (CoreUIComponents, CoreMessaging,
textinputframework, MSCTF) takes and swallows those routinely - eight consecutive
reports proved to be that noise - while every real crash so far faulted in code
loaded from the game folder. The skip applies only to first-chance access
violations: wild EIPs outside any module, rarer codes (heap corruption, illegal
instruction), and everything from the game folder still report first-chance, and
an exception that actually kills the process always gets a full report from the
unhandled filter, which never skips.

## Reading a report

Each report contains:

- the exception code and faulting address, attributed as `module+offset`
  (`Patrician3.exe` loads at its preferred base `0x400000` with an empty
  relocation table, so game offsets translate 1:1 to disassembly addresses);
  for access violations, whether it was a read, write or execute, of what
  address, and **what that address is** - never mapped, reserved, a guard page,
  or live memory with its protection and containing module. A wild pointer into
  the never-mapped first 64 KB and a write to a freed page are different bugs
- all registers
- a hex dump of the code around EIP, the faulting byte marked with `>`
- a hex dump of 0x100 bytes around each register value that points into readable
  memory - the crash-time contents of whatever structures the faulting code was
  working on. The window starts 0x20 below the register so that a heap block's
  header is included, which is what distinguishes a live allocation from a
  recycled one. Unreadable rows print `??` instead of dropping the register
- the thread's SEH chain from `fs:[0]`, innermost first
- the EBP frame chain
- a scan of the stack for values pointing into module code, each classified:
  `ret` means the bytes before it really do encode a `call` (a return address,
  though possibly a stale one from a completed call), `seh handler` means it came
  from an exception registration record, and an unmarked entry is data. Without
  this the scan mixes all three indistinguishably, and reading a handler address
  as a frame sends an investigation the wrong way
- the loaded module map: load range and **full path** of every mapped image.
  The game ships two DLLs called ddraw - GOG's DirectDraw-to-D3D9 wrapper and
  Ascaron's SGL as `ddraw_Dll.dll` - and loads the system `d3d9.dll` behind them,
  so `module+offset` is ambiguous until the file on disk is named

Reports are appended, numbered per game launch. A report the process survived was
a handled exception - the last report in the file is the crash. The
unhandled-exception filter sees the fault the first-chance handler already wrote,
so rather than a second identical copy it appends one line marking that report as
the fatal one; a *different* unhandled fault is still reported in full.

## Blind spot

Heap-corruption kills go through fail-fast (`int 29`) on modern Windows, which
never enters exception dispatch: a crash that leaves no report points at the
heap. Enabling PageHeap (`gflags /p /enable Patrician3.exe /full`) makes the
corruption fault at the corrupting instruction instead, which this reporter
catches.

## Debug keys

The mod doubles as the debugging toolbox: F9 (with modifiers) and F10 are the
throwaway in-game probe keys, rewritten per investigation (`src/probes.rs`).
**Debug builds only** - a `--release` build compiles none of this in, so a
handed-over crash reporter is reporting-only.
Dispatched through the shared hotkey registry (`hotkeys.dll`); without it the
probes are inert, the crash reporting is unaffected. Currently:

| Key | Probe |
|-|-|
| F9 | Captain-experience census to `_probe1.log` |
| Ctrl+F9 | Pirate-convoy sampling timeline toggle |
| Shift+F9 | Selected-ship struct dump to `_probe1_ship.log` |
| Alt+F9 | Player offices + administrator records to `_probe_admin.log` |
| F10 | Every ship's applied route chain, to DebugView |
| Ctrl+F10 | Goods-dialog per-ware order modes |
