//! The plumbing that gets a page drawn.
//!
//! The building windows dispatch on the selected page twice per frame - once in the
//! per-frame update method (vtable `+0xF4`), once in the draw method (`+0x9C`) - each time
//! loading the page with a 6-byte `mov eax,[reg+offset]` and switching through a jump
//! table bounded by an **unsigned** compare. Page `-1` misses every case, so the window
//! draws nothing but its frame: that is the page a details mod fills. The macro replaces
//! both loads with a jump to a stub that calls a Rust hook with the window (`this`, from
//! the register the load used) and returns the page in eax, then continues into the
//! compare as the original would.

/// Install the two page-load detours and the open hook for one window.
///
/// ```ignore
/// p3_page::details_page_detours! {
///     window: UIChurchWindowPtr,
///     // `mov eax,[esi+0x1D30]` in the draw method
///     draw: { patch: 0x005C98A5, original: [0x8b, 0x86, 0x30, 0x1d, 0x00, 0x00], base: "esi" },
///     // `mov eax,[edi+0x1D30]` in the update method
///     update: { patch: 0x005C9542, original: [0x8b, 0x87, 0x30, 0x1d, 0x00, 0x00], base: "edi" },
///     on_open: on_open,      // unsafe fn(window)
///     on_update: on_update,  // unsafe fn(window, page: i32)
///     on_draw: on_draw,      // unsafe fn(window, page: i32)
/// }
/// ```
///
/// This defines `install_page_detours() -> Result<(), u32>`, which verifies the six bytes at
/// each patch site against `original` before writing anything - a different game build
/// fails loudly instead of corrupting code - hooks the window's open slot, and deploys the
/// two detours. `Err` carries the step that failed, for `start()` to return. `on_open` is
/// the place for [crate::page::prepare_drawing_state]; `on_update` for [crate::Page::invalidate]
/// and per-frame state, `on_draw` for the drawing, each when `page == -1`.
///
/// Invoke it once per crate: it defines global symbols.
#[macro_export]
macro_rules! details_page_detours {
    (
        window: $window:ty,
        draw: { patch: $draw_patch:expr, original: $draw_original:expr, base: $draw_base:literal },
        update: { patch: $update_patch:expr, original: $update_original:expr, base: $update_base:literal },
        on_open: $on_open:path,
        on_update: $on_update:path,
        on_draw: $on_draw:path $(,)?
    ) => {
        static __PAGE_DRAW_CONTINUATION: u32 = $draw_patch + 6;
        static __PAGE_UPDATE_CONTINUATION: u32 = $update_patch + 6;
        static __PAGE_OPEN_HOOK: ::std::sync::atomic::AtomicPtr<$crate::hooklet::windows::x86::FunctionPointerHook> =
            ::std::sync::atomic::AtomicPtr::new(::std::ptr::null_mut());

        /// Verify the page loads, hook the window's open method, detour both loads.
        pub unsafe fn install_page_detours() -> Result<(), u32> {
            use $crate::p3_api::ui::page_window::PageWindow;
            let sites: [(u32, [u8; 6]); 2] = [($draw_patch, $draw_original), ($update_patch, $update_original)];
            for (address, expected) in sites {
                let found = *(address as *const [u8; 6]);
                if found != expected {
                    $crate::log::error!("unexpected bytes at the page dispatch {address:#010x}: {found:02x?} - not patching");
                    return Err(1);
                }
            }
            match $crate::hooklet::windows::x86::hook_function_pointer(
                <$window as PageWindow>::VTABLE_OFFSET + $crate::p3_api::ui::page_window::SLOT_OPEN as u32,
                __page_open_hook as *const () as usize as u32,
            ) {
                Ok(hook) => __PAGE_OPEN_HOOK.store(Box::into_raw(Box::new(hook)), ::std::sync::atomic::Ordering::SeqCst),
                Err(_) => {
                    $crate::log::error!("failed to hook the window's open method");
                    return Err(2);
                }
            }
            if $crate::hooklet::windows::x86::deploy_rel32_raw(
                $update_patch as _,
                (&__page_update_detour) as *const _ as _,
                $crate::hooklet::windows::x86::X86Rel32Type::Jump,
            )
            .is_err()
            {
                $crate::log::error!("failed to detour the page load in the update method");
                return Err(3);
            }
            if $crate::hooklet::windows::x86::deploy_rel32_raw(
                $draw_patch as _,
                (&__page_draw_detour) as *const _ as _,
                $crate::hooklet::windows::x86::X86Rel32Type::Jump,
            )
            .is_err()
            {
                $crate::log::error!("failed to detour the page load in the draw method");
                return Err(4);
            }
            Ok(())
        }

        unsafe extern "thiscall" fn __page_open_hook(this: u32) {
            use $crate::p3_api::ui::page_window::PageWindow;
            let orig: extern "thiscall" fn(u32) =
                ::std::mem::transmute((*__PAGE_OPEN_HOOK.load(::std::sync::atomic::Ordering::SeqCst)).old_absolute);
            orig(this);
            $on_open(<$window as PageWindow>::from_address(this));
        }

        unsafe extern "C" fn __page_update_hook(this: u32) -> i32 {
            use $crate::p3_api::ui::page_window::PageWindow;
            let window = <$window as PageWindow>::from_address(this);
            let page = window.selected_page();
            $on_update(window, page);
            page
        }

        unsafe extern "C" fn __page_draw_hook(this: u32) -> i32 {
            use $crate::p3_api::ui::page_window::PageWindow;
            let window = <$window as PageWindow>::from_address(this);
            let page = window.selected_page();
            $on_draw(window, page);
            page
        }

        extern "C" {
            static __page_update_detour: ::std::ffi::c_void;
            static __page_draw_detour: ::std::ffi::c_void;
        }

        // Each stub stands in for `mov eax,[base+offset]`: pass `base` (the window) to the
        // hook, which returns the page in eax, then continue into the compare. ecx and edx are
        // preserved although caller-saved: the game did not expect a call here.
        ::std::arch::global_asm!(
            concat!(
                ".global {update_detour}\n",
                "{update_detour}:\n",
                "push ecx\n",
                "push edx\n",
                "push ", $update_base, "\n",
                "call {update_hook}\n",
                "add esp, 4\n",
                "pop edx\n",
                "pop ecx\n",
                "jmp [{update_continuation}]\n",
                ".global {draw_detour}\n",
                "{draw_detour}:\n",
                "push ecx\n",
                "push edx\n",
                "push ", $draw_base, "\n",
                "call {draw_hook}\n",
                "add esp, 4\n",
                "pop edx\n",
                "pop ecx\n",
                "jmp [{draw_continuation}]\n",
            ),
            update_detour = sym __page_update_detour,
            update_hook = sym __page_update_hook,
            update_continuation = sym __PAGE_UPDATE_CONTINUATION,
            draw_detour = sym __page_draw_detour,
            draw_hook = sym __page_draw_hook,
            draw_continuation = sym __PAGE_DRAW_CONTINUATION,
        );
    };
}
