//! Assets shared by the desktop runner and its packaged executable.

use std::path::PathBuf;

pub type AppResult<T> = Result<T, Box<dyn std::error::Error>>;

/// Reads the packaged guest, falling back to the workspace development build.
pub fn guest(platform: &str) -> AppResult<Vec<u8>> {
	let executable = std::env::current_exe()?;
	let packaged = executable
		.parent()
		.ok_or("the executable has no parent directory")?
		.join("assets/app.wasm");

	match std::fs::read(&packaged) {
		Ok(wasm) => return Ok(wasm),
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
		Err(error) => {
			return Err(format!("could not read {}: {error}", packaged.display()).into());
		}
	}

	let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
		.join("..")
		.join(".leptoskit")
		.join(platform)
		.join("assets/app.wasm");

	std::fs::read(&path)
		.map_err(|error| format!("could not read {}: {error}", path.display()).into())
}
