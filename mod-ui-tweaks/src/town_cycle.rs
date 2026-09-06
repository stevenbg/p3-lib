//! PAGE UP / PAGE DOWN: step through the towns the player may enter, from inside a town.
//! HOME: enter the home town, from a town or from the world map. Registration lives in
//! [crate::ffi::start].

use p3_api::{
    battle::{BattlePoolPtr, NO_BATTLE_SLOT},
    game_world::GAME_WORLD_PTR,
    operations::OPERATIONS_PTR,
    town::get_town_name,
    ui::{ui_local_map_window::UILocalMapWindowPtr, ui_scrollmap_window::UIScrollmapWindowPtr, widget, window_manager::close_open_windows},
};
use windows::Win32::UI::Input::KeyboardAndMouse::VK_NEXT;

/// PAGE DOWN enters the next town the player may enter, PAGE UP the previous one, in town
/// index order and wrapping around. "May enter" is the scrollmap's own double-click test
/// ([p3_api::game_world::GameWorldPtr::can_merchant_enter_town]): a ship of his in the
/// town, or an office.
///
/// Declines (0) - leaving the keys to the game - unless the local-map scene is shown with a
/// town's map in it and the player is not in a sea battle. The map id alone is not enough:
/// it keeps naming the last town on the world map, and a town attacked from the sea fights
/// on the town's own map, so the tests are the scene's visibility and the battle pool's
/// player slot.
pub(crate) unsafe extern "C" fn cycle_town_hotkey(vk: u32, _mods: u32) -> u32 {
    let Some(window) = UILocalMapWindowPtr::new() else {
        return 0;
    };
    if !window.is_shown() || in_battle() {
        return 0;
    }
    let Some(current) = window.get_town_index() else {
        return 0;
    };
    let player = OPERATIONS_PTR.get_player_merchant_index();
    if player < 0 {
        return 0;
    }

    let towns_count = GAME_WORLD_PTR.get_towns_count().min(0xff) as u8;
    let enterable: Vec<u8> = (0..towns_count)
        .filter(|&town| GAME_WORLD_PTR.can_merchant_enter_town(player as u16, town))
        .collect();
    let forward = vk == VK_NEXT.0 as u32;
    let target = if forward {
        enterable.iter().copied().find(|&town| town > current).or_else(|| enterable.first().copied())
    } else {
        enterable.iter().rev().copied().find(|&town| town < current).or_else(|| enterable.last().copied())
    };
    let Some(target) = target else {
        crate::ffi::notify("no town you can enter");
        return 1;
    };
    if target == current {
        crate::ffi::notify(&format!("{}: the only town you can enter", town_name(current)));
        return 1;
    }

    enter(&window, target, false);
    let position = enterable.iter().position(|&town| town == target).map_or(0, |p| p + 1);
    crate::ffi::notify(&format!("{} -> {} ({position} of {} towns)", town_name(current), town_name(target), enterable.len()));
    1
}

/// HOME enters the player's home town (`merchant+0x19`) from the town view or from the
/// world map - what the game does itself when a session starts (`0x00433979`..
/// `0x00433A52`), including its own refusal while the player is in a battle.
///
/// Declines (0) in a battle and without a player; consumes the key otherwise.
pub(crate) unsafe extern "C" fn go_home_hotkey(_vk: u32, _mods: u32) -> u32 {
    let Some(window) = UILocalMapWindowPtr::new() else {
        return 0;
    };
    if in_battle() {
        return 0;
    }
    let player = OPERATIONS_PTR.get_player_merchant_index();
    if player < 0 {
        return 0;
    }
    let home = GAME_WORLD_PTR.get_merchant(player as u16).get_hometown_index();
    if home as u16 >= GAME_WORLD_PTR.get_towns_count() {
        return 0;
    }

    let from_world_map = !window.is_shown();
    if !from_world_map && window.get_town_index() == Some(home) {
        crate::ffi::notify(&format!("{}: already home", town_name(home)));
        return 1;
    }

    enter(&window, home, from_world_map);
    crate::ffi::notify(&format!("home: {}", town_name(home)));
    1
}

/// Is the player taking part in a sea battle? The battle pool's player slot, the test the
/// game's own home-town entry makes (`0x00433979`).
unsafe fn in_battle() -> bool {
    BattlePoolPtr::new().player_battle_slot() != NO_BATTLE_SLOT
}

fn town_name(town: u8) -> String {
    get_town_name(town).unwrap_or_else(|| format!("town {town}"))
}

/// Put `town` on screen. The open windows go first, the way any opening window closes the
/// others. From inside a town the loader alone does it, with the scrollmap's arguments. From
/// the world map the scene has to be swapped as the home-town entry does it
/// (`0x004339A1`..`0x00433A52`): reload, since the map kept in the hidden scene is not the
/// one on screen; centre the world map on the town for the way back; hide and disable the
/// scrollmap; show and enable the local map.
unsafe fn enter(window: &UILocalMapWindowPtr, town: u8, from_world_map: bool) {
    close_open_windows(false);
    window.enter_town(town, from_world_map, true);
    if from_world_map {
        let scrollmap = UIScrollmapWindowPtr::new();
        scrollmap.center_on_town(town);
        widget::show(scrollmap.address, false);
        widget::set_enabled(scrollmap.address, false);
        widget::show(window.address, true);
        widget::set_enabled(window.address, true);
    }
}
