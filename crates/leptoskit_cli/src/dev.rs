//! `leptoskit dev`: watch, rebuild, and serve, with short feedback loops.

use std::net::IpAddr;
use std::net::SocketAddr;
use std::path::Path;
use std::path::PathBuf;
use std::process::Child;
use std::process::Command;
use std::sync::mpsc;
use std::sync::mpsc::RecvTimeoutError;
use std::time::Instant;

use anyhow::Context;
use anyhow::Result;
use notify::RecursiveMode;
use notify::Watcher;

use crate::cli::REBUILD_DEBOUNCE;
use crate::server::LeptosEnv;
use crate::server::SiteEnv;

/// The output directory `leptoskit dev` builds into, relative to the root.
const DEV_OUT: &str = "dist";

/// Builds the terminal app as `leptoskit build --platform tui` does, then runs
/// it in this terminal until it exits.
fn terminal(config: &leptoskit_config::LeptoskitConfig, root: &Path) -> Result<()> {
	config.ensure_platform_enabled(leptoskit_config::Platform::Tui)?;
	crate::native::run(crate::native::NativeCommand::Build {
		target: crate::native::Target::Tui,
		release: false,
	})?;

	let application = crate::native::application(root, config, crate::native::Target::Tui);
	let status = Command::new(&application)
		.status()
		.with_context(|| format!("could not run {}", application.display()))?;
	anyhow::ensure!(status.success(), "the terminal app exited with {status}");
	Ok(())
}

/// Runs the dev loop: an initial build, the server bound on the configured
/// address, then a watcher that rebuilds once changes settle.
///
/// Which server depends on `app.render`. Client-rendered apps get leptoskit's
/// static server over `dist/`. Server-rendered apps (`ssr`, `islands`) run
/// their own server binary with the `LEPTOS_*` environment pointing at
/// `dist/`, restarted after every successful rebuild.
pub fn run(
	open: bool,
	port: Option<u16>,
	platform: Option<leptoskit_config::Platform>,
) -> Result<()> {
	let (config, root) = leptoskit_config::LeptoskitConfig::load()?;
	let platform = platform.unwrap_or(config.app.default_platform);

	if platform == leptoskit_config::Platform::Tui {
		anyhow::ensure!(
			!open && port.is_none(),
			"--open and --port serve the web; the terminal app runs in this terminal"
		);
		return terminal(&config, &root);
	}

	anyhow::ensure!(
		matches!(
			platform,
			leptoskit_config::Platform::Web | leptoskit_config::Platform::Server
		),
		"leptoskit dev serves web applications and runs terminal apps; use `leptoskit build \
		 --platform {platform}` for native builds",
	);
	let port = port.unwrap_or(config.serve.port);
	let ip: IpAddr = config.serve.address.parse().with_context(|| {
		format!(
			"invalid serve.address `{}`: expected an IP address such as 127.0.0.1",
			config.serve.address
		)
	})?;
	let address = SocketAddr::new(ip, port);
	// Bind first, so a taken port fails before the initial build rather than
	// leaving a dev loop with no server behind it.
	let listener = std::net::TcpListener::bind(address)
		.with_context(|| format!("could not bind {address}"))?;
	let server_rendered = config.app.render.renders_on_server()
		&& config.app.default_platform == leptoskit_config::Platform::Web;
	// The app's own server binds the port itself.
	let listener = if server_rendered {
		drop(listener);
		None
	} else {
		listener
			.set_nonblocking(true)
			.context("could not configure the listener")?;
		Some(listener)
	};

	let output = crate::build::run(false, None, None, DEV_OUT)?;

	let metadata = crate::wasm::cargo_metadata(&root)?;
	let out_dir = root.join(DEV_OUT);
	let excluded = vec![
		metadata.target_directory.clone(),
		out_dir.clone(),
		root.join(".leptoskit"),
	];
	let public = root.join("public");

	let (events_tx, events_rx) = mpsc::channel::<()>();
	let filter = WatchFilter {
		public: public.clone(),
		assets: vec![root.join("web"), root.join("assets")],
		excluded: excluded.clone(),
		// Each build rewrites the routes and translations modules when their
		// sources changed; those writes must not trigger another build.
		generated: config
			.router
			.as_ref()
			.map(|router| root.join(router.output()))
			.into_iter()
			.chain(config.l10n.as_ref().map(|l10n| root.join(&l10n.output)))
			.collect(),
	};
	let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
		match event {
			Ok(event) if event.paths.iter().any(|path| filter.is_relevant(path)) => {
				let _ = events_tx.send(());
			}
			Ok(_) => {}
			Err(error) => eprintln!("watch error: {error}"),
		}
	})
	.context("could not start the file watcher")?;

	let mut recursive_roots = Vec::new();
	for package in &metadata.packages {
		if let Some(dir) = package.manifest_path.parent() {
			watch_tree(&mut watcher, dir, &excluded, &mut recursive_roots)?;
		}
	}
	if public.is_dir() && !recursive_roots.iter().any(|dir| public.starts_with(dir)) {
		watch_tree(&mut watcher, &public, &excluded, &mut recursive_roots)?;
	}
	for file in [
		metadata.workspace_root.join("Cargo.toml"),
		root.join(leptoskit_config::CONFIG_FILE),
	] {
		watcher
			.watch(&file, RecursiveMode::NonRecursive)
			.with_context(|| format!("could not watch {}", file.display()))?;
	}

	// Held for the whole loop: dropping the runtime would stop the static
	// server.
	let runtime = tokio::runtime::Runtime::new().context("could not start the async runtime")?;
	let mut app_server = None;
	if let Some(listener) = listener {
		let router = leptoskit_axum::dev_router(&leptoskit_axum::ServeOptions::new(
			&out_dir,
			ip.to_string(),
			port,
		));
		let listener = {
			let _guard = runtime.enter();
			tokio::net::TcpListener::from_std(listener).context("could not adopt the listener")?
		};
		runtime.spawn(async move {
			if let Err(error) = axum::serve(listener, router).await {
				eprintln!("dev server stopped: {error}");
			}
		});
		println!("serving {} on http://{address}", out_dir.display());
	} else {
		app_server = Some(start_app_server(&config, &output, address)?);
		println!("running the app server on http://{address}");
	}

	let url = format!("http://{address}");
	println!("watching {} for changes", root.display());
	if open && let Err(error) = open_browser(&url) {
		eprintln!("could not open the browser: {error:#}");
	}

	loop {
		if events_rx.recv().is_err() {
			break;
		}
		// Debounce: keep absorbing events until none arrives for a whole
		// window, so an editor's burst of writes triggers one rebuild.
		loop {
			match events_rx.recv_timeout(REBUILD_DEBOUNCE) {
				Ok(()) => {}
				Err(RecvTimeoutError::Timeout) => break,
				Err(RecvTimeoutError::Disconnected) => return Ok(()),
			}
		}
		// Stop the app server before cargo replaces its binary: Windows
		// refuses to overwrite a running executable.
		if let Some(mut child) = app_server.take() {
			stop(&mut child);
		}
		let started = Instant::now();
		match crate::build::run(false, None, None, DEV_OUT) {
			Ok(output) => {
				println!("rebuilt in {:?}", started.elapsed());
				if server_rendered {
					match start_app_server(&config, &output, address) {
						Ok(child) => app_server = Some(child),
						Err(error) => eprintln!("could not restart the app server: {error:#}"),
					}
				}
			}
			Err(error) => eprintln!("rebuild failed: {error:#}"),
		}
	}
	Ok(())
}

/// Starts the server binary a build produced, in the leptos `DEV`
/// environment.
fn start_app_server(
	config: &leptoskit_config::LeptoskitConfig,
	output: &crate::build::BuildOutput,
	address: SocketAddr,
) -> Result<Child> {
	let binary = output.server_binary.as_deref().with_context(|| {
		format!(
			"app.render = \"{}\" runs the server package's binary, but `{}` built none (or more \
			 than one, none named after the package)",
			config.app.render, config.app.name
		)
	})?;
	let site = SiteEnv {
		output_name: crate::server::output_name(output.client.as_ref(), &config.app.name),
		site_root: output.out_dir.clone().into_std_path_buf(),
		address,
		env: LeptosEnv::Dev,
	};
	crate::server::spawn(binary, &site)
}

/// Stops a child process and reaps it. A child that already exited is fine.
fn stop(child: &mut Child) {
	let _ = child.kill();
	let _ = child.wait();
}

/// Decides which watched paths trigger a rebuild.
struct WatchFilter {
	public: PathBuf,
	assets: Vec<PathBuf>,
	excluded: Vec<PathBuf>,
	/// The generated routes and translations modules, when the project has
	/// them.
	generated: Vec<PathBuf>,
}

impl WatchFilter {
	fn is_relevant(&self, path: &Path) -> bool {
		if self.excluded.iter().any(|dir| path.starts_with(dir))
			|| self.generated.iter().any(|file| file == path)
		{
			return false;
		}
		// Every public asset is copied into the output, whatever its type.
		path.starts_with(&self.public)
			|| self
				.assets
				.iter()
				.any(|directory| path.starts_with(directory))
			|| matches!(
				path.extension().and_then(|extension| extension.to_str()),
				Some("rs" | "toml" | "lock" | "css" | "html" | "js" | "wasm" | "arb")
			)
	}
}

/// Watches `dir` recursively, except for the `excluded` directories (cargo's
/// target directory and the output) beneath it: those change on every build,
/// and watching them would make each build trigger the next.
fn watch_tree(
	watcher: &mut impl Watcher,
	dir: &Path,
	excluded: &[PathBuf],
	recursive_roots: &mut Vec<PathBuf>,
) -> Result<()> {
	if excluded.iter().any(|excluded| dir.starts_with(excluded))
		|| recursive_roots.iter().any(|root| dir.starts_with(root))
	{
		return Ok(());
	}
	if !excluded.iter().any(|excluded| excluded.starts_with(dir)) {
		watcher
			.watch(dir, RecursiveMode::Recursive)
			.with_context(|| format!("could not watch {}", dir.display()))?;
		recursive_roots.push(dir.to_path_buf());
		return Ok(());
	}
	watcher
		.watch(dir, RecursiveMode::NonRecursive)
		.with_context(|| format!("could not watch {}", dir.display()))?;
	let entries =
		std::fs::read_dir(dir).with_context(|| format!("could not list {}", dir.display()))?;
	for entry in entries {
		let entry = entry.with_context(|| format!("could not list {}", dir.display()))?;
		let hidden = entry.file_name().to_string_lossy().starts_with('.');
		if !hidden && entry.file_type().is_ok_and(|kind| kind.is_dir()) {
			watch_tree(watcher, &entry.path(), excluded, recursive_roots)?;
		}
	}
	Ok(())
}

/// Opens `url` in the default browser with the platform's opener.
fn open_browser(url: &str) -> Result<()> {
	let mut command = if cfg!(target_os = "macos") {
		Command::new("open")
	} else if cfg!(target_os = "windows") {
		let mut command = Command::new("cmd");
		// `start` treats its first quoted argument as a window title.
		command.args(["/C", "start", ""]);
		command
	} else {
		Command::new("xdg-open")
	};
	let program = command.get_program().display().to_string();
	let status = command
		.arg(url)
		.status()
		.with_context(|| format!("could not run {program}"))?;
	anyhow::ensure!(status.success(), "{program} exited with {status}");
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	fn filter(generated: &[&str]) -> WatchFilter {
		WatchFilter {
			public: PathBuf::from("/app/public"),
			assets: vec![PathBuf::from("/app/web"), PathBuf::from("/app/assets")],
			excluded: vec![PathBuf::from("/app/target"), PathBuf::from("/app/dist")],
			generated: generated.iter().map(PathBuf::from).collect(),
		}
	}

	#[test]
	fn rebuilds_for_sources_and_public_assets_only() {
		let filter = filter(&[]);
		assert!(filter.is_relevant(Path::new("/app/src/lib.rs")));
		assert!(filter.is_relevant(Path::new("/app/locales/pt-BR.arb")));
		assert!(filter.is_relevant(Path::new("/app/public/logo.png")));
		assert!(filter.is_relevant(Path::new("/app/assets/logo.png")));
		assert!(filter.is_relevant(Path::new("/app/web/favicon.ico")));
		assert!(!filter.is_relevant(Path::new("/app/src/notes.txt")));
		assert!(!filter.is_relevant(Path::new("/app/target/debug/app.rs")));
		assert!(!filter.is_relevant(Path::new("/app/dist/pkg/app.wasm")));
	}

	#[test]
	fn the_generated_modules_never_trigger_a_rebuild() {
		let filter = filter(&["/app/src/routes.rs", "/app/src/l10n.rs"]);
		assert!(!filter.is_relevant(Path::new("/app/src/routes.rs")));
		assert!(!filter.is_relevant(Path::new("/app/src/l10n.rs")));
		assert!(filter.is_relevant(Path::new("/app/src/routes/index.rs")));
		assert!(filter.is_relevant(Path::new("/app/locales/en.arb")));
	}
}
