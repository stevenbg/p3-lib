#![allow(clippy::missing_safety_doc)]

extern crate num_derive;

pub mod auto_trader;
pub mod class35;
pub mod data;
pub mod facility;
pub mod game_world;
pub mod letters;
pub mod merchant;
pub mod missions;
pub mod mods;
pub mod operation;
pub mod operations;
pub mod scheduled_tasks;
pub mod ship;
pub mod ships;
pub mod town;
pub mod ui;

/// The mapped trade difficulty (2.2 low, 2.0 normal, 1.8 high): the selling price
/// curve's factor at market stock 0. Field +0x64 of the static class at 0x006DE3D8
/// that every get_sell_price (0x0052E1D0) caller passes as `this`.
pub const TRADE_DIFFICULTY_ADDRESS: *const f32 = 0x006DE43C as _;

#[derive(Clone, Debug)]
#[repr(C)]
pub struct Point<T> {
    pub x: T,
    pub y: T,
}

impl<T> Point<T> {
    pub const fn new(x: T, y: T) -> Self {
        Self { x, y }
    }
}

// https://stackoverflow.com/a/28175593/1569755
fn latin1_to_string(s: &[u8]) -> String {
    s.iter().take_while(|c| **c != 0).map(|&c| c as char).collect()
}

pub unsafe fn latin1_ptr_to_string(mut s: *const u8) -> String {
    let mut result = String::new();
    while *s != 0 {
        result.push(*s as char);
        s = s.add(1);
    }
    result
}

pub unsafe fn free(address: u32) {
    let orig: extern "cdecl" fn(ptr: u32) = std::mem::transmute(0x0063A1B6);
    orig(address);
}
