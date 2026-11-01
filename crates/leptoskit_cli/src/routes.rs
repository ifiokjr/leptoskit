//! `leptoskit routes`: the forwarding alias for `plugin:router routes`, which
//! writes the typed `Route` module from the routes directory. `build`, `dev`
//! and `native` run the same generation first, through the plugin registry,
//! so the module is current before cargo compiles it.

use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use leptoskit_config::LeptoskitConfig;

/// Runs `leptoskit routes`: forwards to the router plugin's `routes` command,
/// writing the module, or with `check`, failing when the committed one is out
/// of date.
pub fn command(check: bool) -> Result<()> {
	let (config, root) = LeptoskitConfig::load()?;
	crate::build::router_spec(&config).with_context(|| {
		format!(
			"the file router is off; activate it with a `[[plugins]]` table naming `router` in {} \
			 (a `[router]` table still works)",
			leptoskit_config::CONFIG_FILE
		)
	})?;
	crate::ui::warning(
		"`routes` moved to `plugin:router routes`; the top-level command forwards and will be \
		 removed",
	);
	let registry = crate::build::resolve_plugins(&config)?;
	let mut arguments = Vec::new();

	if check {
		arguments.push("--check".to_owned());
	}

	registry.run_command(&root, &config, "router", "routes", &arguments)?;

	Ok(())
}

/// Brings the generated module up to date when the project uses the file
/// router, before a build compiles it. Writes only when the routes changed,
/// so the dev watcher never sees a rewrite of identical content.
pub fn sync(root: &Path, config: &LeptoskitConfig) -> Result<()> {
	crate::build::generate_outputs(root, config)?;
	Ok(())
}
