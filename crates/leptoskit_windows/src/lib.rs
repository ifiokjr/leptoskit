//! <!-- {=windowsSummary|trim|linePrefix:"//! ":true} -->
//!
//! The backend owns its HWNDs on one UI thread and consumes the existing Yoga
//! mutation stream; it never embeds a browser or creates a second layout tree.
//! The executable includes the provided manifest for version 6 common controls
//! and layered-window support.
//!
//! <!-- {/windowsSummary} -->
//!
//! Each element owns a Win32 child window. Interactive elements additionally
//! own a standard Windows control, preserving native keyboard and accessibility
//! behavior. The backend and its windows stay on their creating UI thread.
//! This crate is empty on other operating systems.
#![allow(unsafe_code)]

#[cfg(target_os = "windows")]
mod accessibility;
#[cfg(target_os = "windows")]
mod backend;
#[cfg(target_os = "windows")]
mod drawing;
#[cfg(target_os = "windows")]
mod events;

#[cfg(target_os = "windows")]
mod host;
#[cfg(target_os = "windows")]
mod rive;
#[cfg(target_os = "windows")]
mod vector;

#[cfg(target_os = "windows")]
pub use backend::WindowsBackend;
#[cfg(target_os = "windows")]
pub use drawing::WindowsTextMeasurer;
#[cfg(target_os = "windows")]
pub use events::EventListener;
#[cfg(target_os = "windows")]
pub use host::WindowsWindow;
