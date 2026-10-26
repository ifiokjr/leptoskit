#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![cfg_attr(all(feature = "nightly", rustc_nightly), feature(fn_traits))]
#![cfg_attr(all(feature = "nightly", rustc_nightly), feature(unboxed_closures))]
#![cfg_attr(all(feature = "nightly", rustc_nightly), allow(unstable_features))]
//! Cross-platform Leptos utilities with explicit host services.
//!
//! Pure hooks use the same signals as `leptos::prelude`. Host-dependent hooks
//! take explicit inputs: a monotonic time signal, a viewport signal, or a
//! [`Storage`] or [`AsyncStorage`] implementation. Pure hooks require no DOM
//! or executor. The optional `local-storage` feature supplies persistent JSON
//! adapters for native application directories and browser local storage.
//!
//! Create hooks inside a Leptos owner. Watchers run synchronously on every
//! platform and stop on owner cleanup. Timer callbacks run when the host
//! advances its clock. Async storage futures are polled by the host executor;
//! native file adapters offload blocking I/O to worker threads.
//!
//! ```
//! use leptoskit_use::UseToggleReturn;
//! use leptoskit_use::use_toggle;
//! use reactive_graph::owner::Owner;
//! use reactive_graph::prelude::*;
//!
//! let owner = Owner::new();
//! owner.with(|| {
//! 	let UseToggleReturn { value, toggle, .. } = use_toggle(false);
//! 	toggle();
//! 	assert!(value.get());
//! });
//! ```

mod async_state;
mod async_storage;
mod derived;
#[cfg(all(feature = "nightly", rustc_nightly))]
mod nightly;
#[cfg(feature = "local-storage")]
mod persistent;
mod state;
mod storage;
mod time;
mod viewport;
mod watch;

pub use async_state::*;
pub use async_storage::*;
pub use derived::*;
#[cfg(feature = "local-storage")]
pub use persistent::*;
pub use state::*;
pub use storage::*;
pub use time::*;
pub use viewport::*;
pub use watch::*;

/// A cloneable action, suitable for event handlers on any platform.
pub type Action = std::sync::Arc<dyn Fn() + Send + Sync>;
