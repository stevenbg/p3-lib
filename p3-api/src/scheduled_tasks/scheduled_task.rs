use crate::{data::p3_ptr::P3Pointer, missions::alderman_missions::AldermanMissionPtr};

pub const SCHEDULED_TASK_SIZE: u32 = 0x18;
pub const SCHEDULED_TASK_OPCODE_ALDERMAN_MISSION: u16 = 0x32;
/// The ten-day world update (`0x004DDA40`, rescheduled `due += 0xA00` by the dispatcher
/// at `0x004D8668`). Among much else it runs the captain scan `0x004DCEA0`, which keeps
/// its round counter in this task's own data at `+0x8`.
pub const SCHEDULED_TASK_OPCODE_TEN_DAY_UPDATE: u16 = 0x03;

pub struct ScheduledTaskPtr {
    address: u32,
}

pub enum ScheduledTaskData {
    AldermanMission(AldermanMissionPtr),
    TODO,
}

impl ScheduledTaskPtr {
    pub fn new(address: u32) -> Self {
        Self { address }
    }

    pub unsafe fn get_due_timestamp(&self) -> u32 {
        self.get(0x00)
    }

    pub unsafe fn get_next_task_index(&self) -> u16 {
        self.get(0x04)
    }

    pub unsafe fn get_opcode(&self) -> u16 {
        self.get(0x06)
    }

    /// A raw dword out of the 16-byte data union at `+0x8`.
    pub unsafe fn get_data_dword(&self, offset: u32) -> u32 {
        self.get(0x08 + offset)
    }

    pub unsafe fn get_data(&self) -> Option<ScheduledTaskData> {
        let data_address = self.address + 0x08;
        let opcode = self.get_opcode();
        match opcode {
            SCHEDULED_TASK_OPCODE_ALDERMAN_MISSION => Some(ScheduledTaskData::AldermanMission(AldermanMissionPtr::new(data_address))),
            _ => None,
        }
    }
}

impl P3Pointer for ScheduledTaskPtr {
    fn get_address(&self) -> u32 {
        self.address
    }
}
