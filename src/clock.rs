//! The monotonic clock the runtime classes read (`Instant`, `Timer.time:`).
//!
//! `std::time::Instant::now()` unconditionally panics on wasm32-unknown-unknown (no
//! OS clock), so the browser build swaps in `web_time::Instant` — identical API,
//! backed by `performance.now()` through wasm-bindgen. Native builds re-export std
//! untouched. Import the clock from here in any module that is compiled for wasm;
//! native-only code (the drivers, the smol backend) keeps using std directly.

#[cfg(not(target_arch = "wasm32"))]
pub use std::time::Instant;
#[cfg(target_arch = "wasm32")]
pub use web_time::Instant;
