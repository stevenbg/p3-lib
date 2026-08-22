use std::mem;

use windows::Win32::System::Memory::{VirtualQuery, MEMORY_BASIC_INFORMATION, MEM_COMMIT, PAGE_GUARD, PAGE_NOACCESS};

/// The highest address this process can map. `Patrician3.exe` is not
/// `LARGE_ADDRESS_AWARE` (PE characteristics `0x010F`), so the user-mode half of the
/// address space ends at 2 GiB and no valid game pointer is ever at or above it.
pub const USER_ADDRESS_LIMIT: u32 = 0x8000_0000;

/// True if `len` bytes at `address` are committed and readable.
///
/// Pointers read out of live game state have to be checked before they are followed:
/// a field can be zero before its owning object is populated, and nothing guarantees
/// the target is still mapped. A range test alone only rejects obvious garbage (zero,
/// small integers, `0xFFFFFFFF`); this also rejects addresses whose pages are reserved,
/// freed or guarded, which is what actually crashes.
pub unsafe fn is_readable(address: u32, len: usize) -> bool {
    if address == 0 || address >= USER_ADDRESS_LIMIT {
        return false;
    }
    let mut info: MEMORY_BASIC_INFORMATION = mem::zeroed();
    if VirtualQuery(Some(address as _), &mut info, mem::size_of::<MEMORY_BASIC_INFORMATION>()) == 0 {
        return false;
    }
    if info.State != MEM_COMMIT || (info.Protect & (PAGE_NOACCESS | PAGE_GUARD)).0 != 0 {
        return false;
    }
    // The queried region describes one protection run; a read may not cross its end.
    let region_end = info.BaseAddress as u32 as u64 + info.RegionSize as u64;
    address as u64 + len as u64 <= region_end
}
