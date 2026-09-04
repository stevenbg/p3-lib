pub(crate) mod ffi;
// The debug probes exist only in debug builds: `debug_assertions` is on for
// `cargo build` and off for `--release`, so a handed-over release DLL carries no
// probe code at all.
#[cfg(debug_assertions)]
pub(crate) mod probes;
#[cfg(debug_assertions)]
pub(crate) mod scroll_window;
