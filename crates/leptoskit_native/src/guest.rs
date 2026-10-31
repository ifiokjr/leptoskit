//! Render portable elements inside the native host's Wasm interpreter.
//!
//! The application supplies the same tree it uses on the web. This adapter
//! sends its original styles and classes to the host, which owns layout and
//! native widgets. Event handlers stay in the guest and run before the tree is
//! rendered again. Enable `guest` for native Wasm bundles, never browser
//! bundles.

#[cfg(any(target_arch = "wasm32", test))]
mod compare;
#[cfg(any(target_arch = "wasm32", test))]
mod tree;

#[cfg(target_arch = "wasm32")]
mod runtime;

#[cfg(target_arch = "wasm32")]
pub use runtime::alloc;
#[cfg(target_arch = "wasm32")]
pub use runtime::dispatch;
#[cfg(target_arch = "wasm32")]
pub use runtime::mount;
#[cfg(target_arch = "wasm32")]
pub use runtime::render;
#[cfg(target_arch = "wasm32")]
pub use runtime::set_viewport;
#[cfg(target_arch = "wasm32")]
pub use runtime::viewport;

/// The native UI ABI implemented by this guest adapter.
pub const ABI_VERSION: u32 = 1;
