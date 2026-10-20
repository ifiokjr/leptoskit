#![forbid(unsafe_code)]
#![deny(missing_docs)]
//! Cross-platform Leptos utilities, with no DOM or executor requirement.
//!
//! Pure hooks use the same signals as `leptos::prelude`. Host-dependent hooks
//! take explicit inputs: a monotonic time signal, a viewport signal, or a
//! [`Storage`] implementation. They never silently become no-ops on native or
//! SSR.
//!
//! Create hooks inside a Leptos owner. Watchers run synchronously on every
//! platform and stop on owner cleanup. Timer callbacks run when the host
//! advances its clock; no global runtime, background thread, or browser window
//! is assumed.
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

mod derived;
mod state;
mod storage;
mod time;
mod viewport;
mod watch;

pub use derived::*;
pub use state::*;
pub use storage::*;
pub use time::*;
pub use viewport::*;
pub use watch::*;

/// A cloneable action, suitable for event handlers on any platform.
pub type Action = std::sync::Arc<dyn Fn() + Send + Sync>;
