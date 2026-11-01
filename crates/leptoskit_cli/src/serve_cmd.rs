//! `leptoskit serve`: serves the built output.
//!
//! Client-rendered apps are static files, served by leptoskit's own server.
//! Server-rendered apps (`app.render = "ssr"` or `"islands"`) serve
//! themselves: `serve` runs the server binary the last `leptoskit build`
//! produced, in the leptos `PROD` environment, pointed at the built site.

use std::net::IpAddr;
use std::net::SocketAddr;

use anyhow::Context;
use anyhow::Result;
use camino::Utf8PathBuf;

use crate::server::BuiltServer;
use crate::server::LeptosEnv;
use crate::server::SiteEnv;

/// Serves `dir` using the project configuration's address and port.
pub fn run(dir: &str, port: Option<u16>) -> Result<()> {
	let (config, root) = leptoskit_config::LeptoskitConfig::load()?;
	let root = Utf8PathBuf::from_path_buf(root)
		.map_err(|_| anyhow::anyhow!("project path is not UTF-8"))?;
	let dir_path = root.join(dir);
	anyhow::ensure!(
		dir_path.is_dir(),
		"{dir_path} does not exist; run `leptoskit build` first"
	);
	crate::build::serve_plugins(&config)?;
	let port = port.unwrap_or(config.serve.port);
	// A client-rendered app with a server package is served by that server,
	// which answers its API too, as `leptoskit dev` does.
	if config.app.render.renders_on_server() || has_built_server(&root) {
		return run_app_server(&config, &root, dir_path, port);
	}
	let options = leptoskit_axum::ServeOptions::new(
		dir_path.as_std_path(),
		config.serve.address.clone(),
		port,
	);
	crate::ui::success(format_args!(
		"serving {dir_path} at {}http://{}:{port}{}",
		crate::ui::BOLD.render(),
		config.serve.address,
		crate::ui::BOLD.render_reset()
	));
	let runtime = tokio::runtime::Runtime::new().context("could not start the async runtime")?;
	runtime.block_on(async {
		leptoskit_axum::serve(options)
			.await
			.map_err(|error| anyhow::anyhow!("{error}"))
	})
}

/// Whether the last build produced a server binary.
fn has_built_server(root: &Utf8PathBuf) -> bool {
	crate::wasm::cargo_metadata(root.as_std_path()).is_ok_and(|metadata| {
		BuiltServer::load(&metadata.target_directory, root.as_std_path()).is_ok()
	})
}

/// Runs the last build's server binary in the foreground until it exits.
fn run_app_server(
	config: &leptoskit_config::LeptoskitConfig,
	root: &Utf8PathBuf,
	site_root: Utf8PathBuf,
	port: u16,
) -> Result<()> {
	let ip: IpAddr = config.serve.address.parse().with_context(|| {
		format!(
			"invalid serve.address `{}`: expected an IP address such as 127.0.0.1",
			config.serve.address
		)
	})?;
	let metadata = crate::wasm::cargo_metadata(root.as_std_path())?;
	let server = BuiltServer::load(&metadata.target_directory, root.as_std_path())?;
	let client = crate::wasm::detect_client_with(
		config.build.client_package.as_deref(),
		Some(&config.app.name),
		&metadata,
	)?;
	let site = SiteEnv {
		output_name: crate::server::output_name(client.as_ref(), &config.app.name),
		site_root: site_root.into_std_path_buf(),
		address: SocketAddr::new(ip, port),
		env: LeptosEnv::Prod,
	};
	crate::ui::success(format_args!(
		"running {} at {}http://{}{} ({} render)",
		server.binary.display(),
		crate::ui::BOLD.render(),
		site.address,
		crate::ui::BOLD.render_reset(),
		config.app.render
	));
	let status = crate::server::spawn(&server.binary, &site)?
		.wait()
		.context("could not wait for the app server")?;
	anyhow::ensure!(status.success(), "the app server exited with {status}");
	Ok(())
}
