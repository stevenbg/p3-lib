use std::mem;

use ui_scrollmap_window::UIScrollmapWindowPtr;

use crate::data::{screen_rectangle::Rect, ui_render_text_at};

pub mod animation;
pub mod button;
pub mod class73;
pub mod custom_window;
pub mod ddraw;
pub mod font;
pub mod graphics;
pub mod number_widget;
pub mod page_window;
pub mod rich_text;
pub mod scroll_list;
pub mod tooltip;
pub mod ui_church_window;
pub mod ui_event_window;
pub mod ui_notifications;
pub mod ui_local_map_window;
pub mod ui_main_scene;
pub mod ui_scrollmap_window;
pub mod ui_ship_panel;
pub mod ui_shipyard_window;
pub mod ui_tavern_window;
pub mod ui_town_hall_sidemenu;
pub mod ui_town_hall_window;
pub mod ui_trading_office_window;
pub mod widget;
pub mod window_manager;

pub unsafe fn rect_clipper_stuff(rect: *const Rect) {
    let function: extern "stdcall" fn(rect: *const Rect) = mem::transmute(0x004B9650);
    function(rect)
}

pub fn draw_geometry(x1: i32, y1: i32, x2: i32, y2: i32) {
    let function: extern "cdecl" fn(x1: i32, y1: i32, x2: i32, y2: i32) = unsafe { mem::transmute(0x004BD680) };
    function(x1, y1, x2, y2)
}

pub unsafe fn draw_geometry_abs(x1: i32, y1: i32, x2: i32, y2: i32) {
    let scrollmap = UIScrollmapWindowPtr::new();
    let function: extern "cdecl" fn(x1: i32, y1: i32, x2: i32, y2: i32) = unsafe { mem::transmute(0x004BD680) };
    function(
        x1 - scrollmap.get_offset_x() as i32 + scrollmap.get_x(),
        y1 - scrollmap.get_offset_y() as i32 + scrollmap.get_y(),
        x2 - scrollmap.get_offset_x() as i32 + scrollmap.get_x(),
        y2 - scrollmap.get_offset_y() as i32 + scrollmap.get_y(),
    )
}

pub unsafe fn draw_text_at_abs(x: i32, y: i32, text: &[u8]) {
    let scrollmap = UIScrollmapWindowPtr::new();
    ui_render_text_at(
        x - scrollmap.get_offset_x() as i32 + scrollmap.get_x(),
        y - scrollmap.get_offset_y() as i32 + scrollmap.get_y(),
        text,
    )
}
