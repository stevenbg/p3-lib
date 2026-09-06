use std::{ffi::c_void, mem::transmute};

use log::debug;

use crate::{data::p3_ptr::P3Pointer, operation::Operation};

pub const OPERATIONS_PTR: OperationsPtr = OperationsPtr::new();
const OPERATIONS_ADDRESS: u32 = 0x006DF2F0;
/// The value of the speed level that means fast forward.
pub const FAST_FORWARD_LEVEL: u32 = 1;
/// `ops+0x92C`, the speed level, as an absolute address - for assembly operands, where a
/// method call is not available.
pub const GAME_SPEED_LEVEL_ADDRESS: u32 = OPERATIONS_ADDRESS + 0x092C;
static EXECUTE_OPERATION_ADDRESS: u32 = 0x00535760;
static ENQUEUE_OPERATION_ADDRESS: u32 = 0x0054AA70;
static TRANSFER_LOADED_TRADEROUTE_ADDRESS: u32 = 0x005492D0;
static ENQUEUE_OPERATION: &extern "thiscall" fn(*mut c_void, *const c_void) = unsafe { transmute(&ENQUEUE_OPERATION_ADDRESS) };
static TRANSFER_LOADED_TRADEROUTE: &extern "thiscall" fn(*mut c_void) = unsafe { transmute(&TRANSFER_LOADED_TRADEROUTE_ADDRESS) };

#[derive(Clone, Debug, Copy)]
pub struct OperationsPtr {
    pub address: u32,
}

impl Default for OperationsPtr {
    fn default() -> Self {
        Self::new()
    }
}

impl OperationsPtr {
    pub const fn new() -> Self {
        Self { address: OPERATIONS_ADDRESS }
    }

    pub unsafe fn get_status(&self) -> u32 {
        self.get(0x00)
    }

    pub unsafe fn set_current_operations_array_pos(&self, pos: u16) {
        self.set(0x474 + 0x0e, &pos)
    }

    /// `ops+0x924` (`0x006DFC14`). Written by the drain's opcode `0xC1` case
    /// (`0x00546978`..`0x005469A1`): the last merchant whose control word is 0, a human. Not
    /// yet valid when the load routine ([crate::save_game::LOAD_ADDRESS]) returns - that
    /// operation runs later in the session start.
    pub unsafe fn get_player_merchant_index(&self) -> i32 {
        self.get(0x0924)
    }

    /// The game speed level: `0` normal play, `1` fast forward, `2` local map. See
    /// [crate::operation::Operation::SetGameSpeed] for the tick pacer that reads it.
    pub unsafe fn get_game_speed_level(&self) -> u32 {
        self.get(0x092C)
    }

    /// Whether the world is running in fast forward (level `1`).
    pub unsafe fn is_fast_forward(&self) -> bool {
        self.get_game_speed_level() == FAST_FORWARD_LEVEL
    }

    pub unsafe fn get_unpacked_traderoute_ptr(&self) -> *mut c_void {
        self.get(0x930)
    }

    pub unsafe fn enqueue_operation(&self, op: Operation) {
        debug!("Enqueuing operation {op:?}");
        let op_bytes = op.to_raw();
        ENQUEUE_OPERATION(OPERATIONS_ADDRESS as _, op_bytes.as_ptr() as _)
    }

    pub unsafe fn transfer_loaded_traderoute(&self) {
        debug!("Transfer loaded traderoute");
        TRANSFER_LOADED_TRADEROUTE(OPERATIONS_ADDRESS as _)
    }
}

impl P3Pointer for OperationsPtr {
    fn get_address(&self) -> u32 {
        self.address
    }
}

pub fn execute_operation(op: &Operation) {
    debug!("execute_operation({:?})", op);
    let op = op.to_raw();
    let execute_operation: extern "thiscall" fn(op: *const u8) = unsafe { transmute(EXECUTE_OPERATION_ADDRESS) };
    execute_operation(op.as_ptr());
}
