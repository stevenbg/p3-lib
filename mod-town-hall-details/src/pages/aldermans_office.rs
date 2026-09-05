use std::ffi::CStr;

use log::warn;
use p3_api::{
    data::enums::FacilityId,
    game_world::GAME_WORLD_PTR,
    missions::alderman_missions::{AldermanMissionDataPtr, FoundTownPtr},
    operations::OPERATIONS_PTR,
    scheduled_tasks::{scheduled_task::ScheduledTaskData, SCHEDULED_TASKS_PTR},
    ui::ui_town_hall_window::UITownHallWindowPtr,
};
use p3_page::{Cell, Column, Page, Table};

/// Under the window's title banner, in the game's own 20 px pitch for this window.
const FIRST_ROW_Y: i32 = 60;
const ROW_HEIGHT: i32 = 20;
/// Labels from the left, values ending at the right column.
const COLUMNS: [Column; 2] = [Column::left(25, 300), Column::right(400, 300)];

static TASK_RESCHEDULING_IN: &CStr = c"Rescheduling in";
static TASK_RESCHEDULES_REMAINING: &CStr = c"Reschedule Counter";
static TOWN: &CStr = c"Town";
static EFFECTIVE_PRODUCTION: &CStr = c"Effective Production";
static LOW_PRODUCTION: &CStr = c"Low Production";

pub(crate) unsafe fn draw_page(window: UITownHallWindowPtr) {
    let next_mission_index = window.get_next_mission_index();
    if next_mission_index == 0 {
        return;
    }

    if SCHEDULED_TASKS_PTR.get_merchant_alderman_mission_task_index(OPERATIONS_PTR.get_player_merchant_index()) != -1 {
        return;
    }

    let selected_mission_index = window.get_selected_alderman_mission_index();
    if selected_mission_index == 0xff {
        return;
    }
    let task_index = window.get_task_index(selected_mission_index);
    let task = SCHEDULED_TASKS_PTR.get_scheduled_task(task_index);
    let data = match task.get_data() {
        Some(e) => e,
        None => return,
    };

    let mission = match data {
        ScheduledTaskData::AldermanMission(mission) => mission,
        _ => return,
    };

    let mut page = Page::new(&window, FIRST_ROW_Y);
    page.row_height = ROW_HEIGHT;
    page.reset_state();
    let table = Table::new(&page, COLUMNS);
    let mut y = page.top;

    let rescheduling_in = task.get_due_timestamp() - GAME_WORLD_PTR.get_game_time_raw();
    y = table.row(y, &[TASK_RESCHEDULING_IN.into(), (rescheduling_in as i32).into()]);
    y = table.row(y, &[TASK_RESCHEDULES_REMAINING.into(), (mission.get_reschedule_counter() as i32).into()]);

    match mission.get_data() {
        AldermanMissionDataPtr::FoundTownPtr(ptr) => draw_found_town(&table, y, &ptr),
        AldermanMissionDataPtr::OverlandTradeRoute(_ptr) => {}
        AldermanMissionDataPtr::NotoriousPirate(_ptr) => {}
        AldermanMissionDataPtr::PirateHideout(_ptr) => {}
        AldermanMissionDataPtr::SupplyProblems(_ptr) => {}
    }
}

/// The found-town mission's terms: the town, what will produce well there, and what will
/// not. The fisherman's house reads as whale oil when the descriptor's `0x20000` bit is set,
/// in which case fish is the low production.
unsafe fn draw_found_town(table: &Table, y: i32, data: &FoundTownPtr) {
    let town = data.get_town();
    let effective_raw = data.get_production_effective_raw();
    let mut y = table.row(y, &[TOWN.into(), format!("{town:?}").into()]);

    let whale_oil = effective_raw & 0x20000 != 0;
    let mut effective: Vec<&str> = Vec::new();
    for facility in data.get_production_effective() {
        let ware = match facility {
            FacilityId::Militia | FacilityId::Shipyard | FacilityId::Construction | FacilityId::Weaponsmith => {
                warn!("Unexpected facility {facility:?}");
                continue;
            }
            FacilityId::HuntingLodge => "Skins",
            FacilityId::FishermansHouse => {
                if whale_oil {
                    "Whale Oil"
                } else {
                    "Fish"
                }
            }
            FacilityId::Brewery => "Beer",
            FacilityId::Workshop => "Iron Goods",
            FacilityId::Apiary => "Honey",
            FacilityId::GrainFarm => "Grain",
            FacilityId::CattleFarm => "Meat, Leather",
            FacilityId::Sawmill => "Timber",
            FacilityId::WeavingMill => "Cloth",
            FacilityId::Saltery => "Salt",
            FacilityId::Ironsmelter => "Pig Iron",
            FacilityId::SheepFarm => "Wool",
            FacilityId::Vineyard => "Wine",
            FacilityId::Pottery => "Pottery",
            FacilityId::Brickworks => "Bricks",
            FacilityId::Pitchmaker => "Pitch",
            FacilityId::HempFarm => "Hemp",
        };
        effective.push(ware);
    }
    y = table.row(y, &[EFFECTIVE_PRODUCTION.into(), effective.join(", ").into()]);

    let low = if whale_oil { Cell::from("Fish") } else { Cell::Empty };
    table.row(y, &[LOW_PRODUCTION.into(), low]);
}
