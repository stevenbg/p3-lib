use crate::{data::p3_ptr::P3Pointer, missions::alderman_missions::AldermanMissionPtr};

pub const SCHEDULED_TASK_SIZE: u32 = 0x18;
pub const SCHEDULED_TASK_OPCODE_ALDERMAN_MISSION: u16 = 0x32;
/// One per town: the yearly mayor election (`0x004DBB00`). Task data `+0x8` (u16) is the
/// town's election day of the year, `+0xE` the town index; on that day it runs
/// `0x00528E60(town)` and re-arms for `((year * 365 + day) << 8) | 0xFA`.
pub const SCHEDULED_TASK_OPCODE_MAYOR_ELECTION: u16 = 0x02;
/// The ten-day world update (`0x004DDA40`, rescheduled `due += 0xA00` by the dispatcher
/// at `0x004D8668`). Among much else it runs the captain scan `0x004DCEA0`, which keeps
/// its round counter in this task's own data at `+0x8`.
pub const SCHEDULED_TASK_OPCODE_TEN_DAY_UPDATE: u16 = 0x03;
/// The daily weather pass (`0x004E4984`). Outside the ice season it runs `0x004F34A4`;
/// when the day of the year is `<= 58` or `>= 333` it runs the ice pass `0x004E45C4`,
/// which is what freezes ports.
pub const SCHEDULED_TASK_OPCODE_DAILY_WEATHER: u16 = 0x0D;
/// Thaw one port: `0x004E94A4` clears [crate::town::TOWN_FLAG_FROZEN] on the town whose
/// index sits in this task's data at `+0x8` and posts "The port of %s is open again.".
/// The ice pass schedules it `(ice_level & 0x7F) + 1` days out when a port freezes.
pub const SCHEDULED_TASK_OPCODE_UNFREEZE_PORT: u16 = 0x35;

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
