//! Render modes: the cargo features each side of a web build gets.
//!
//! leptos apps switch rendering through cargo features, by a convention the
//! whole ecosystem shares: the server package has an `ssr` feature, the wasm
//! client a `hydrate` feature, and islands are the `islands` feature of
//! `leptos` itself. `app.render` picks the mode once, in `leptoskit.toml`,
//! and the build turns it into features for each side, so the server and the
//! client can never disagree about how the page renders.

use anyhow::Result;
use leptoskit_config::RenderMode;

use crate::wasm::MetadataPackage;

/// The server package's feature for rendering HTML.
pub const SSR_FEATURE: &str = "ssr";
/// The client package's feature for hydrating server-rendered HTML.
pub const HYDRATE_FEATURE: &str = "hydrate";

/// The mode features for each side of a build.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModeFeatures {
	/// Features for the server build.
	pub server: Vec<String>,
	/// Features for the wasm client build.
	pub client: Vec<String>,
}

/// The features `mode` adds to the server package and, when the build has
/// one, the client package.
///
/// # Errors
///
/// Returns an error naming the missing piece when a package cannot render in
/// `mode`: a server-rendering mode needs the server's `ssr` feature and the
/// client's `hydrate` feature, and islands need both packages to reach
/// `leptos`, directly or through `leptoskit`.
pub fn mode_features(
	mode: RenderMode,
	server: &MetadataPackage,
	client: Option<&MetadataPackage>,
) -> Result<ModeFeatures> {
	if !mode.renders_on_server() {
		return Ok(ModeFeatures::default());
	}
	let mut features = ModeFeatures {
		server: vec![required_feature(mode, server, SSR_FEATURE, "server")?],
		client: Vec::new(),
	};
	if let Some(client) = client {
		features
			.client
			.push(required_feature(mode, client, HYDRATE_FEATURE, "client")?);
	}
	if mode == RenderMode::Islands {
		features.server.push(islands_feature(server)?);
		if let Some(client) = client {
			features.client.push(islands_feature(client)?);
		}
	}
	Ok(features)
}

fn required_feature(
	mode: RenderMode,
	package: &MetadataPackage,
	feature: &str,
	side: &str,
) -> Result<String> {
	anyhow::ensure!(
		package.declares_feature(feature),
		"app.render = \"{mode}\" builds the {side} package `{name}` with its `{feature}` feature, \
		 but `{name}` declares no such feature; add `{feature} = [\"leptoskit/{feature}\"]` (or \
		 `[\"leptos/{feature}\"]`, plus your other {side}-only dependencies) under [features] in \
		 its Cargo.toml, or set app.render = \"csr\"",
		name = package.name,
	);
	Ok(feature.to_owned())
}

/// The `islands` feature of leptos, spelled with the package's own name for
/// the dependency it reaches leptos through: `leptos` itself when the package
/// depends on it directly, otherwise `leptoskit`, whose `islands` feature
/// forwards to it.
fn islands_feature(package: &MetadataPackage) -> Result<String> {
	let via = package
		.dependency_key("leptos")
		.or_else(|| package.dependency_key("leptoskit"))
		.ok_or_else(|| {
			anyhow::anyhow!(
				"app.render = \"islands\" turns on `leptos/islands` for `{}`, but it depends on \
				 neither leptos nor leptoskit directly; add one of them to its [dependencies]",
				package.name
			)
		})?;
	Ok(format!("{via}/islands"))
}
