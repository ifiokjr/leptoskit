//! Generate or check the public control document schema.

use std::path::Path;

use leptoskit_testing::control;

fn main() -> Result<(), Box<dyn std::error::Error>> {
	let check = match std::env::args().nth(1).as_deref() {
		None => false,
		Some("--check") => true,
		Some(_) => return Err("usage: schema [--check]".into()),
	};
	let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/src");
	let asset = "schemas/control.schema.json";
	let id = format!("https://ifiokjr.github.io/leptoskit/{asset}");
	let schema = control::document_schema(&id);
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
			Ok(_) => {
				return Err(
					format!("stale schema: {asset}; run `devenv shell schema:update`").into(),
				);
			}
			Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
				return Err(
					format!("missing schema: {asset}; run `devenv shell schema:update`").into(),
				);
			}
			Err(error) => return Err(error.into()),
		}
	} else {
		if let Some(parent) = path.parent() {
			std::fs::create_dir_all(parent)?;
		}
		std::fs::write(&path, bytes)?;
		println!("updated {}", path.display());
	}

	Ok(())
}
