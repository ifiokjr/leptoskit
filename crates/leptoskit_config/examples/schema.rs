//! Generate or check the public schema aliases and current major's contract.

use std::path::Path;

use leptoskit_config::SCHEMA_VERSION;
use leptoskit_config::schema;

fn main() -> Result<(), Box<dyn std::error::Error>> {
	let check = match std::env::args().nth(1).as_deref() {
		None => false,
		Some("--check") => true,
		Some(_) => return Err("usage: schema [--check]".into()),
	};

	let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/src");
	let versioned = format!("schemas/leptoskit.v{SCHEMA_VERSION}.schema.json");
	let assets = ["schema.json", "schemas/leptoskit.schema.json", &versioned];
	let mut stale = Vec::new();

	for asset in assets {
		let id = format!("https://ifiokjr.github.io/leptoskit/{asset}");
		let schema = schema::configuration(&id);
		// Match the repository's dprint JSON format for deterministic checks.
		let formatter = serde_json::ser::PrettyFormatter::with_indent(b"\t");
		let mut serializer = serde_json::Serializer::with_formatter(Vec::new(), formatter);
		serde::Serialize::serialize(&schema, &mut serializer)?;
		let mut bytes = serializer.into_inner();
		bytes.push(b'\n');

		let path = root.join(asset);

		if check {
			match std::fs::read(&path) {
				Ok(existing) if existing == bytes => {}
				Ok(_) => stale.push(asset),

				Err(error) if error.kind() == std::io::ErrorKind::NotFound => stale.push(asset),
				Err(error) => return Err(error.into()),
			}
		} else {
			if let Some(parent) = path.parent() {
				std::fs::create_dir_all(parent)?;
			}

			std::fs::write(&path, bytes)?;
			println!("updated {}", path.display());
		}
	}

	if !stale.is_empty() {
		return Err(format!(
			"stale schemas: {}; run `devenv shell schema:update`",
			stale.join(", ")
		)
		.into());
	}

	Ok(())
}
