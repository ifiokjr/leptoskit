//! `leptoskit serve`: runs the static server over the built output.

use anyhow::Context;
use anyhow::Result;
use camino::Utf8PathBuf;

/// Serves `dir` using the project configuration's address and port.
pub fn run(dir: &str, port: Option<u16>) -> Result<()> {
	let (config, root) =
		leptoskit_config::LeptoskitConfig::load().context("no leptoskit project found")?;
	let root = Utf8PathBuf::from_path_buf(root)
		.map_err(|_| anyhow::anyhow!("project path is not UTF-8"))?;
	let dir_path = root.join(dir);
	anyhow::ensure!(
		dir_path.is_dir(),
		"{dir_path} does not exist; run `leptoskit build` first"
	);
	let port = port.unwrap_or(config.serve.port);
	let options = leptoskit_axum::ServeOptions::new(
		dir_path.as_std_path(),
		config.serve.address.clone(),
		port,
	);
	println!(
		"serving {dir_path} on http://{}:{port}",
		config.serve.address
	);
	let runtime = tokio::runtime::Runtime::new().context("could not start the async runtime")?;
	runtime.block_on(async {
		leptoskit_axum::serve(options)
			.await
			.map_err(|error| anyhow::anyhow!("{error}"))
	})
}
