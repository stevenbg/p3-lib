//! A right-click on the Options button (the cogs under the minimap) opens the mod's own
//! window ([crate::config_window]). Installed from [crate::ffi::start].
//!
//! The game routes a right-button release to the pressed and the focused child of the main
//! scene, never to the child under the cursor, so the button itself never sees it. The
//! capture is therefore on the scene's right-button-up slot
//! ([UIMainScenePtr::SLOT_RIGHT_BUTTON_UP]): a release inside the button's rectangle is
//! taken here and not passed on, so the game does not also treat it as the right-click that
//! closes the topmost window; everything else goes to the game's own handler.

use std::{
    mem,
    sync::atomic::{AtomicPtr, Ordering},
};

use hooklet::windows::x86::{hook_function_pointer, FunctionPointerHook};
use log::error;
use p3_api::ui::{ui_main_scene::UIMainScenePtr, widget};

static RIGHT_UP_HOOK: AtomicPtr<FunctionPointerHook> = AtomicPtr::new(std::ptr::null_mut());

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
                crate::config_window::toggle();
                return;
            }
        }
    }
    let original: extern "thiscall" fn(u32, u32, i32, i32) = mem::transmute((*RIGHT_UP_HOOK.load(Ordering::SeqCst)).old_absolute);
    original(scene, flags, x, y);
}
