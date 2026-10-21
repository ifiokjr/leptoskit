//! Real Android Views for the shared leptoskit native tree.
//!
//! Compile `android/WidgetHost.kt` into the application and construct an
//! `AndroidBackend` from its instance on the Activity's UI thread. The host
//! owns native views; Rust owns reconciliation, layout, and typed actions.
//! Android widget APIs are available only when targeting Android.
#![allow(unsafe_code)]

#[cfg(target_os = "android")]
mod backend;
#[cfg(all(target_os = "android", feature = "probe"))]
mod probe;

#[cfg(target_os = "android")]
pub use backend::AndroidBackend;
#[cfg(target_os = "android")]
pub use backend::AndroidError;
