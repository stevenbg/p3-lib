//! The mod's own window, opened by a right-click on the Options button
//! ([crate::options_button]): where the mod's configuration will live. Built on
//! `p3_api::ui::custom_window` with a game scrollbar from `scroll_list`, and until it has
//! settings to show it lists placeholder rows - the living example of both.
use std::sync::atomic::{AtomicU32, Ordering};

use log::{error, info};
use p3_api::{
    data::{ddraw_set_constant_color, screen_rectangle::Rect, ui_render_text_at},
    ui::{
        custom_window::{DrawContext, GameWindow, WindowContent},
        font::{self, TextMode},
        scroll_list::ScrollList,
    },
};

const WINDOW_X: i32 = 300;
const WINDOW_Y: i32 = 140;
const WINDOW_W: i32 = 512;
const WINDOW_H: i32 = 320;
const ROW_COUNT: u32 = 60;
const VISIBLE_ROWS: i32 = 12;
const ROW_HEIGHT: i32 = 18;
const LIST_X: i32 = 40;
const LIST_Y: i32 = 50;
/// Right edge of the list area (where the bar sits), from the window's right edge.
const BAR_MARGIN: i32 = 40;
const TEXT_COLOR: u32 = 0xff00_0000;
const TITLE: &[u8] = b"Trading QoL";

/// The window's object address once built; 0 before the first open.
static WINDOW: AtomicU32 = AtomicU32::new(0);

struct Rows {
    list: ScrollList,
}

impl WindowContent for Rows {
    fn draw(&mut self, _window: &GameWindow, ctx: &DrawContext) {
        let first = self.list.first_row();
        font::ddraw_set_font(font::get_normal_font());
        font::ddraw_set_text_mode(TextMode::AlignLeft);
        ddraw_set_constant_color(TEXT_COLOR);
        for i in 0..self.list.visible_rows() {
            let row = first + i;
            if row >= ROW_COUNT as i32 {
                break;
            }
            let mut text = format!("row {row:02}").into_bytes();
            text.push(0);
            unsafe { ui_render_text_at(ctx.x + LIST_X, ctx.y + LIST_Y + i * ROW_HEIGHT, &text) };
        }
    }

    fn on_close(&mut self, _window: &GameWindow) {
        info!("config window: closed");
    }
}

/// Open the window, or close it when it is open.
pub(crate) unsafe fn toggle() {
    let window = match WINDOW.load(Ordering::SeqCst) {
        0 => {
            let window = build();
            WINDOW.store(window.address, Ordering::SeqCst);
            window
        }
        address => GameWindow { address },
    };
    if window.is_open() {
        window.close();
    } else if window.open() {
        info!("config window: opened at {},{} {}x{}", window.x(), window.y(), window.width(), window.height());
    } else {
        error!("config window: no scene root to join");
        crate::ffi::notify("Trading QoL: no scene to open the window in");
    }
}

unsafe fn build() -> GameWindow {
    let list = ScrollList::new();
    let window = GameWindow::new(WINDOW_X, WINDOW_Y, WINDOW_W, WINDOW_H, Box::new(Rows { list }));
    window.set_title(TITLE);
    // The list area: text on the left, bar against its right edge; the wheel works anywhere
    // inside it.
    let top = window.y() + LIST_Y;
    let area = Rect {
        left: window.x() + LIST_X - 8,
        top,
        right: window.x() + window.width() - BAR_MARGIN,
        bottom: top + VISIBLE_ROWS * ROW_HEIGHT,
    };
    list.set_area(area, ROW_HEIGHT);
    list.set_count(ROW_COUNT);
    window.add_widget(Box::new(list));
    info!("config window: built {:#x}, list {:#x}", window.address, list.address);
    window
}
