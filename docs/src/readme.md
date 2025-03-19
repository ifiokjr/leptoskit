# Introduction

This is the `leptoskit` book.

Currently the best way to get started with leptos is to use `cargo-leptos`. While there is some great engineering involved I don't find it easy enough to get started with, or to extend. Also it doesn't seem to leverage the rust ecosystem as much as it could.

Here's what `leptoskit` offers.

1. Configuration is placed in a `build.rs` file rather than a `Cargo.toml` file. This provides type checking and discoverable configuration options when first getting started.

```rust
// build.rs

use leptoskit::config::Config;
use leptoskit_auth::plugin::AuthPlugin;
use leptoskit_routes::plugin::FileRoutesPlugin;
use leptoskit_skribble::plugin::SkribblePlugin;

fn main() {
	// Create a new configuration
	let mut config = Config::default();
	let mut auth_plugin = AuthPlugin::default();
	let mut routes_plugin = FileRoutesPlugin::default();
	let mut skribble_plugin = FileRoutesPlugin::default();

	// Add the plugins to the configuration
	config.add_plugin(auth_plugin);
	config.add_plugin(routes_plugin);
	config.add_plugin(skribble_plugin);

	// Save the configuration
	config.save();
}
```

The above file will setup up `leptoskit` with file_routes and auth plugins.

1. Extensibility via build and runtime plugins.

Plugins are loaded during the build time. For example the `Routes` plugin provides a file router from the `routes` directory. The plugin is able to read the crate structure and extract all routes based on the configuration provided to the plugin. This is all run at build time.

For runtime there are macros available for example `#[leptoskit::page]` which is used to annotate a page component.

The build plugins inject code into the automated build process.

## Current limitations

Leptos supports both `actix` and `axum`.

For ease of development `leptoskit` only supports `axum`.
