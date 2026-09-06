//! The mod's own window, opened by a right-click on the Options button (the cogs under the
//! minimap): where the mod's configuration will live. Built on `p3_api::ui::custom_window`
//! with a game scrollbar from `scroll_list`, and until it has settings to show it lists
//! placeholder rows - the living example of both.
//!
//! The opener is a hook installed from [crate::ffi::start] ([install]). The game routes a
//! right-button release to the pressed and the focused child of the main scene, never to the
//! child under the cursor, so the button itself never sees it. The capture is therefore on
//! the scene's right-button-up slot ([UIMainScenePtr::SLOT_RIGHT_BUTTON_UP]): a release
//! inside the button's rectangle is taken here and not passed on, so the game does not also
//! treat it as the right-click that closes the topmost window; everything else goes to the
//! game's own handler.
use std::{
    mem,
    sync::atomic::{AtomicPtr, AtomicU32, Ordering},
};

use hooklet::windows::x86::{hook_function_pointer, FunctionPointerHook};
use log::{error, info};
use p3_api::{
    data::{ddraw_set_constant_color, screen_rectangle::Rect, ui_render_text_at},
    ui::{
        custom_window::{DrawContext, GameWindow, WindowContent},
        font::{self, TextMode},
        scroll_list::ScrollList,
        ui_main_scene::UIMainScenePtr,
        widget,
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
static RIGHT_UP_HOOK: AtomicPtr<FunctionPointerHook> = AtomicPtr::new(std::ptr::null_mut());

/// Hook the main scene's right-button-up slot so a right-click on the Options button
/// toggles the window. `Err(step)` names the failed step: 1 = the slot does not hold the
/// game's handler, 2 = the hook could not be installed.
pub(crate) unsafe fn install() -> Result<(), u32> {
    let slot = 0x0040_0000 + UIMainScenePtr::VTABLE_OFFSET + UIMainScenePtr::SLOT_RIGHT_BUTTON_UP;
    let current = *(slot as *const u32);
    if current != UIMainScenePtr::RIGHT_BUTTON_UP_ADDRESS {
        error!("main scene right-button-up slot reads {current:#010x}, expected {:#010x} - not hooking", UIMainScenePtr::RIGHT_BUTTON_UP_ADDRESS);
        return Err(1);
    }
    match hook_function_pointer(UIMainScenePtr::VTABLE_OFFSET + UIMainScenePtr::SLOT_RIGHT_BUTTON_UP, right_button_up_hook as *const () as usize as u32) {
        Ok(hook) => {
            RIGHT_UP_HOOK.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst);
            Ok(())
        }
        Err(_) => Err(2),
    }
}

/// `0x004298F0(flags, x, y)`, `ret 0xC`.
unsafe extern "thiscall" fn right_button_up_hook(scene: u32, flags: u32, x: i32, y: i32) {
    if Some(scene) == UIMainScenePtr::new().map(|s| s.address) {
        let button = scene + UIMainScenePtr::OPTIONS_BUTTON_OFFSET;
        if widget::is_visible(button) {
            let (bx, by) = widget::position(button);
            let (bw, bh) = widget::size(button);
            if (bx..bx + bw).contains(&x) && (by..by + bh).contains(&y) {
                toggle();
                return;
            }
        }
    }
    let original: extern "thiscall" fn(u32, u32, i32, i32) = mem::transmute((*RIGHT_UP_HOOK.load(Ordering::SeqCst)).old_absolute);
    original(scene, flags, x, y);
}

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
