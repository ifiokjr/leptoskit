//! `leptoskit serve`: serves the built output.
//!
//! Client-rendered apps are static files, served by leptoskit's own server.
//! Server-rendered apps (`app.render = "ssr"` or `"islands"`) serve
//! themselves: `serve` runs the server binary the last `leptoskit build`
//! produced, in the leptos `PROD` environment, pointed at the built site.
//! The services that build made start first, and when any is public, a proxy
//! on the configured port puts their paths on the app's origin.

use std::net::IpAddr;
use std::net::SocketAddr;

use anyhow::Context;
use anyhow::Result;
use camino::Utf8PathBuf;

use crate::server::BuiltServer;
use crate::server::LeptosEnv;
use crate::server::SiteEnv;
use crate::services::BuiltServices;
use crate::services::proxy::Routes;
use crate::services::run::Running;

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
	let ip: IpAddr = config.serve.address.parse().with_context(|| {
		format!(
			"invalid serve.address `{}`: expected an IP address such as 127.0.0.1",
			config.serve.address
		)
	})?;
	let address = SocketAddr::new(ip, port);
	let runtime = tokio::runtime::Runtime::new().context("could not start the async runtime")?;
	let services = if config.services.is_empty() {
		BuiltServices::default()
	} else {
		let metadata = crate::wasm::cargo_metadata(root.as_std_path())?;
		BuiltServices::load(&metadata.target_directory, root.as_std_path())?
	};
	let routes = Routes::of(&services.services);
	let app_address = if routes.is_empty() {
		address
	} else {
		let free = std::net::TcpListener::bind(SocketAddr::new(ip, 0))
			.context("could not find a free port for the app behind the services' proxy")?;
		free.local_addr()?
	};
	let (running, app_env) = start_services(&config, &root, &services, address, app_address)?;

	if !routes.is_empty() {
		let listener = std::net::TcpListener::bind(address)
			.with_context(|| format!("could not bind {address}"))?;
		listener.set_nonblocking(true)?;
		let listener = {
			let _guard = runtime.enter();
			tokio::net::TcpListener::from_std(listener)?
		};
		let proxy = crate::services::proxy::router(routes, app_address);
		runtime.spawn(async move {
			if let Err(error) = axum::serve(listener, proxy).await {
				crate::ui::warning(format_args!("the services' proxy stopped: {error}"));
			}
		});
	}

	// A client-rendered app with a server package is served by that server,
	// which answers its API too, as `leptoskit dev` does.
	let result = if config.app.render.renders_on_server() || has_built_server(&root) {
		// SIGINT or SIGTERM stops the app server, and then the services, so
		// a process manager stopping `serve` leaves nothing behind.
		let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
		let signalled = std::sync::Arc::clone(&stop);
		runtime.spawn(async move {
			leptoskit_axum::shutdown_signal().await;
			signalled.store(true, std::sync::atomic::Ordering::Relaxed);
		});
		run_app_server(
			&config,
			&root,
			dir_path,
			app_address,
			address,
			&app_env,
			&stop,
		)
	} else {
		let options = leptoskit_axum::ServeOptions::new(
			dir_path.as_std_path(),
			app_address.ip().to_string(),
			app_address.port(),
		);
		crate::ui::success(format_args!(
			"serving {dir_path} at {}http://{address}{}",
			crate::ui::BOLD.render(),
			crate::ui::BOLD.render_reset()
		));
		runtime.block_on(async {
			leptoskit_axum::serve(options)
				.await
				.map_err(|error| anyhow::anyhow!("{error}"))
		})
	};
	drop(running);
	result
}

/// Starts the last build's services, and returns them with the variables
/// the app receives for them.
fn start_services(
	config: &leptoskit_config::LeptoskitConfig,
	root: &Utf8PathBuf,
	services: &BuiltServices,
	address: SocketAddr,
	app_address: SocketAddr,
) -> Result<(Running, Vec<(String, String)>)> {
	if services.services.is_empty() {
		return Ok((Running::default(), Vec::new()));
	}

	let tokens = crate::services::run::tokens(&services.services)?;
	let session = crate::services::run::Session {
		config,
		root: root.as_std_path(),
		public_origin: format!("http://{address}"),
		app_url: format!("http://{app_address}"),
		app_database_url: config
			.db
			.as_ref()
			.and_then(|db| std::env::var(&db.url_env).ok()),
		tokens: &tokens,
	};
	let prepared = crate::services::run::prepare(&services.services, &session)?;
	let running = crate::services::run::start(&prepared, &[], crate::server::SERVE_STOP_GRACE)?;
	Ok((running, prepared.app_env))
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
	app_address: SocketAddr,
	public: SocketAddr,
	env: &[(String, String)],
	stop: &std::sync::atomic::AtomicBool,
) -> Result<()> {
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
		address: app_address,
		env: LeptosEnv::Prod,
	};
	crate::ui::success(format_args!(
		"running {} at {}http://{public}{} ({} render)",
		server.binary.display(),
		crate::ui::BOLD.render(),
		crate::ui::BOLD.render_reset(),
		config.app.render
	));
	crate::server::ensure_port_free(app_address, "the app server")?;
	let mut child = crate::server::spawn_with(&server.binary, &site, env)?;

	loop {
		if let Some(status) = child
			.try_wait()
			.context("could not wait for the app server")?
		{
			anyhow::ensure!(status.success(), "the app server exited with {status}");
			return Ok(());
		}

		if stop.load(std::sync::atomic::Ordering::Relaxed) {
			// It drains the way it would in a deploy, then the services stop.
			crate::server::stop_gracefully(&mut [&mut child], crate::server::SERVE_STOP_GRACE);
			return Ok(());
		}

		std::thread::sleep(std::time::Duration::from_millis(100));
	}
}
