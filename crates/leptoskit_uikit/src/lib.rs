//! Native UIKit rendering for shared leptoskit component trees.
//!
//! Widgets and callback state remain on the main thread. The text measurer
//! creates independent Core Text values for every measurement.
#![allow(unsafe_code)]

#[cfg(target_os = "ios")]
mod backend;
#[cfg(target_os = "ios")]
mod events;
#[cfg(target_os = "ios")]
mod host;
#[cfg(target_os = "ios")]
mod measure;
#[cfg(target_os = "ios")]
mod text;

#[cfg(target_os = "ios")]
pub use backend::UIKitBackend;
#[cfg(target_os = "ios")]
pub use events::EventListener;
#[cfg(target_os = "ios")]
pub use host::UIKitHost;
#[cfg(target_os = "ios")]
pub use host::run_application;
#[cfg(target_os = "ios")]
pub use measure::UIKitTextMeasurer;
#[cfg(target_os = "ios")]
pub use objc2::MainThreadMarker;
