//! The screens and routes `leptoskit generate auth --idp` writes into an
//! app, compiled from the CLI's own templates, so a change to the hooks that
//! breaks them fails here first. The templates are the `--idp` version; the
//! generator leaves those lines out without the flag. Each route file is a
//! module of its own here, where the file router would gather them.

// The screens are the app's code once written, held to the app's lints
// rather than to this repository's pedantic ones: they match on a query's
// data and error together, and take small values by reference like the rest.
#[allow(clippy::manual_let_else, clippy::trivially_copy_pass_by_ref)]
#[path = "../../../crates/leptoskit_cli/src/auth_pages/templates/mod.rs"]
pub mod auth;
#[path = "../../../crates/leptoskit_cli/src/auth_pages/templates/route_connected_apps.rs"]
pub mod route_connected_apps;
#[path = "../../../crates/leptoskit_cli/src/auth_pages/templates/route_consent.rs"]
pub mod route_consent;
#[path = "../../../crates/leptoskit_cli/src/auth_pages/templates/route_developer_apps.rs"]
pub mod route_developer_apps;
#[path = "../../../crates/leptoskit_cli/src/auth_pages/templates/route_device.rs"]
pub mod route_device;
#[path = "../../../crates/leptoskit_cli/src/auth_pages/templates/route_devices.rs"]
pub mod route_devices;
#[path = "../../../crates/leptoskit_cli/src/auth_pages/templates/route_security.rs"]
pub mod route_security;
#[path = "../../../crates/leptoskit_cli/src/auth_pages/templates/route_sign_in.rs"]
pub mod route_sign_in;
#[path = "../../../crates/leptoskit_cli/src/auth_pages/templates/route_sign_in_idp.rs"]
pub mod route_sign_in_idp;
