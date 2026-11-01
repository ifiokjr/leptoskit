//! Render portable elements inside the native host's Wasm interpreter.
//!
//! The application supplies the same tree it uses on the web. This adapter
//! sends its original styles and classes to the host, which owns layout and
//! native widgets. Event handlers stay in the guest. A handler that writes a
//! signal settles the bindings and the [`For`](crate::reactive::For) /
//! [`Show`](crate::reactive::Show) containers that read it, and the view is
//! built again only when the view itself read the signal. Enable `guest` for
//! native Wasm bundles, never browser bundles.

#[cfg(any(target_arch = "wasm32", test))]
mod compare;
#[cfg(any(target_arch = "wasm32", test))]
mod region;
#[cfg(any(target_arch = "wasm32", test))]
mod track;
#[cfg(any(target_arch = "wasm32", test))]
mod tree;

#[cfg(target_arch = "wasm32")]
mod runtime;

#[cfg(target_arch = "wasm32")]
pub use runtime::alloc;
#[cfg(target_arch = "wasm32")]
pub use runtime::dispatch;
#[cfg(target_arch = "wasm32")]
pub use runtime::flush;
#[cfg(target_arch = "wasm32")]
pub use runtime::mount;
#[cfg(target_arch = "wasm32")]
pub use runtime::poll;
#[cfg(target_arch = "wasm32")]
pub use runtime::render;
#[cfg(target_arch = "wasm32")]
pub use runtime::set_viewport;
#[cfg(target_arch = "wasm32")]
pub use runtime::spawn;
#[cfg(target_arch = "wasm32")]
pub use runtime::viewport;

/// The native UI ABI implemented by this guest adapter.
pub const ABI_VERSION: u32 = 1;
