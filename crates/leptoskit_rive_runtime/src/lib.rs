//! <!-- {=rive_runtimeSummary|trim|linePrefix:"//! ":true} -->
//!
//! Applications use `leptoskit_rive`; this crate exposes no application API.
//! Its separate Cargo build lets changes to the Rust player or image adapter
//! reuse the C++ compilation cache.
//!
//! <!-- {/rive_runtimeSummary} -->
//!
//! Applications should depend on `leptoskit_rive`. Keeping this build separate
//! lets player and adapter edits reuse the native archive.
#![no_std]
