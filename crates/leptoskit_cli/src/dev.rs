//! `leptoskit dev`: watch, rebuild, and serve every platform, with short
//! feedback loops.
//!
//! By default it runs every platform `app.platforms` enables that this
//! machine can run: the web app, and each native app on this machine, in a
//! simulator or emulator, or on an attached device. `--platform` narrows that
//! to the platforms named. Every change rebuilds the web app and each native
//! guest, and the native apps swap their new guest in without being
//! reinstalled.
//!
//! With a `[db]` table, it also runs the development database (see
//! `db::dev_server`) and keeps it in step with the tables as they change.

mod guests;
mod native;

use std::net::IpAddr;
use std::net::SocketAddr;
use std::path::Path;
use std::path::PathBuf;
use std::process::Child;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::sync::mpsc::RecvTimeoutError;
use std::time::Instant;

use anyhow::Context;
use anyhow::Result;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::Platform;
use notify::RecursiveMode;
use notify::Watcher;

use crate::cli::REBUILD_DEBOUNCE;
use crate::server::LeptosEnv;
use crate::server::SiteEnv;
use crate::ui;
use crate::ui::BAD;
use crate::ui::BOLD;
use crate::ui::DIM;
use crate::ui::GOOD;

/// The output directory `leptoskit dev` builds into, relative to the root.
const DEV_OUT: &str = "dist";

/// Builds the terminal app as `leptoskit build --platform tui` does, then runs
/// it in this terminal until it exits.
fn terminal(config: &LeptoskitConfig, root: &Path) -> Result<()> {
	config.ensure_platform_enabled(Platform::Tui)?;
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

/// Runs the dev loop: an initial build of every selected platform, then a
/// watcher that rebuilds them once changes settle. `platforms` names the
/// platforms to run; empty means every enabled platform this machine can run.
/// `devices` narrows the mobile platforms to the simulators and devices named.
///
/// The terminal app needs this terminal to itself, so it runs alone: when it
/// is the one platform named, or when nothing is named and it is the app's
/// default platform.
pub fn run(
	open: bool,
	port: Option<u16>,
	platforms: &[Platform],
	devices: &[String],
	no_devtools: bool,
	devtools_port: Option<u16>,
) -> Result<()> {
	let (config, root) = LeptoskitConfig::load()?;

	if terminal_only(&config, &root, platforms, open, port, devices)? {
		return Ok(());
	}

	let tools = native::Tools::from_environment();
	let selection = native::select(&config, platforms, &tools)?;
	report_skipped(&selection);
	anyhow::ensure!(
		selection.web || selection.extension || !selection.native.is_empty(),
		"this machine can run none of the platforms in app.platforms"
	);

	// Held for the whole loop: dropping the runtime would stop the servers.
	let runtime = tokio::runtime::Runtime::new().context("could not start the async runtime")?;
	let started = Started::start(Start {
		config: &config,
		root: &root,
		port,
		devices,
		tools,
		selection: &selection,
		runtime: &runtime,
		no_devtools,
		devtools_port,
	})?;
	let (_watcher, events_rx, config_changed, tables_changed) = watch(
		&config,
		&root,
		&runtime,
		open,
		started.web.as_ref(),
		started.database.as_ref(),
	)?;
	rebuild(started, &events_rx, &config_changed, &tables_changed);
	Ok(())
}

fn terminal_only(
	config: &LeptoskitConfig,
	root: &Path,
	platforms: &[Platform],
	open: bool,
	port: Option<u16>,
	devices: &[String],
) -> Result<bool> {
	let terminal_only = match platforms {
		[] => config.app.default_platform == Platform::Tui,
		platforms => platforms.iter().all(|platform| *platform == Platform::Tui),
	};

	if !terminal_only {
		return Ok(false);
	}

	anyhow::ensure!(
		!open && port.is_none() && devices.is_empty(),
		"--open, --port, and --device choose where other platforms run; the terminal app runs in \
		 this terminal"
	);
	terminal(config, root)?;
	Ok(true)
}

fn report_skipped(selection: &native::Selection) {
	for (platform, reason) in &selection.skipped {
		ui::tagged(platform, format_args!("{DIM}skipped: {reason}{DIM:#}"));
	}
}

struct Started {
	web: Option<Web>,
	/// The development database, stopped after the server that uses it.
	database: Option<crate::db::DevDatabase>,
	extension: Option<crate::extension::dev::ExtensionDev>,
	native: Option<native::NativeDev>,
	_devtools: Option<crate::devtools::Hub>,
}

struct Start<'a> {
	config: &'a LeptoskitConfig,
	root: &'a Path,
	port: Option<u16>,
	devices: &'a [String],
	tools: native::Tools,
	selection: &'a native::Selection,
	runtime: &'a tokio::runtime::Runtime,
	no_devtools: bool,
	devtools_port: Option<u16>,
}

impl Started {
	fn start(start: Start<'_>) -> Result<Self> {
		let Start {
			config,
			root,
			port,
			devices,
			tools,
			selection,
			runtime,
			no_devtools,
			devtools_port,
		} = start;
		let devtools = if no_devtools {
			None
		} else {
			Some(crate::devtools::start(
				root,
				runtime,
				devtools_port,
				port.unwrap_or(config.serve.port),
			)?)
		};

		let database = start_database(config, root)?;
		let database_env = database
			.as_ref()
			.map_or_else(Vec::new, crate::db::DevDatabase::env);
		let web = selection
			.web
			.then(|| Web::start(config, root, port, runtime, database_env))
			.transpose()?;
		let extension = selection
			.extension
			.then(|| crate::extension::dev::ExtensionDev::start(root, runtime))
			.transpose()?;
		let native = if selection.native.is_empty() {
			None
		} else {
			let native =
				native::NativeDev::start(root, config, &selection.native, devices, tools, runtime)?;
			anyhow::ensure!(
				web.is_some() || extension.is_some() || !native.is_empty(),
				"no platform started"
			);
			(!native.is_empty()).then_some(native)
		};

		Ok(Self {
			web,
			database,
			extension,
			native,
			_devtools: devtools,
		})
	}
}

fn watch(
	config: &LeptoskitConfig,
	root: &Path,
	runtime: &tokio::runtime::Runtime,
	open: bool,
	web: Option<&Web>,
	database: Option<&crate::db::DevDatabase>,
) -> Result<(
	notify::RecommendedWatcher,
	mpsc::Receiver<Wake>,
	Arc<AtomicBool>,
	Arc<AtomicBool>,
)> {
	let (events_tx, events_rx) = mpsc::channel::<Wake>();
	let stop = events_tx.clone();
	runtime.spawn(async move {
		leptoskit_axum::shutdown_signal().await;
		let _ = stop.send(Wake::Stop);
	});

	let (mut watcher, config_changed, tables_changed, scope) =
		start_watcher(config, root, events_tx, database)?;
	watch_roots(&mut watcher, &scope)?;

	anstream::println!(
		"{DIM}watching{DIM:#} {} {DIM}for changes{DIM:#}",
		root.display()
	);

	if let Some(web) = web
		&& open
		&& let Err(error) = open_browser(&web.url())
	{
		ui::warning(format_args!("could not open the browser: {error:#}"));
	}

	Ok((watcher, events_rx, config_changed, tables_changed))
}

/// Starts the app's development database, if it has one, and brings it in
/// step with the tables once.
fn start_database(config: &LeptoskitConfig, root: &Path) -> Result<Option<crate::db::DevDatabase>> {
	let database = crate::db::DevDatabase::start(config, root)?;

	if let Some(database) = &database {
		database.tables_changed();
	}

	Ok(database)
}

/// The files the watcher covers.
struct Watched {
	metadata: crate::wasm::CargoMetadata,
	public: PathBuf,
	excluded: Vec<PathBuf>,
	config_file: PathBuf,
}

fn start_watcher(
	config: &LeptoskitConfig,
	root: &Path,
	events_tx: mpsc::Sender<Wake>,
	database: Option<&crate::db::DevDatabase>,
) -> Result<(
	notify::RecommendedWatcher,
	Arc<AtomicBool>,
	Arc<AtomicBool>,
	Watched,
)> {
	let metadata = crate::wasm::cargo_metadata(root)?;
	let out_dir = root.join(DEV_OUT);
	let excluded = excluded_dirs(&root, &metadata, &out_dir);
	let public = root.join("public");
	let config_file = root.join(leptoskit_config::CONFIG_FILE);
	let config_changed = Arc::new(AtomicBool::new(false));
	// The package that declares the tables: a change there may change them.
	let tables_dir = database.and_then(|database| database.package_dir(&metadata));
	let tables_changed = Arc::new(AtomicBool::new(false));

	let filter = WatchFilter {
		public: public.clone(),
		assets: vec![root.join("web"), root.join("assets")],
		excluded: excluded.clone(),
		// Each build rewrites the generated modules when their inputs
		// changed; those writes must not trigger another build. Plugins
		// report theirs; the translations module is hardwired for now.
		generated: crate::build::generate_outputs(root, config)?
			.into_iter()
			.chain(config.l10n.as_ref().map(|l10n| root.join(&l10n.output)))
			.collect(),
	};
	let watched_config = config_file.clone();
	let changed = Arc::clone(&config_changed);
	let tables = Arc::clone(&tables_changed);
	let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
		match event {
			Ok(event) if event.paths.iter().any(|path| filter.is_relevant(path)) => {
				if event.paths.contains(&watched_config) {
					changed.store(true, Ordering::Relaxed);
				}

				if let Some(dir) = &tables_dir
					&& event
						.paths
						.iter()
						.any(|path| path.starts_with(dir) && filter.is_relevant(path))
				{
					tables.store(true, Ordering::Relaxed);
				}

				let _ = events_tx.send(Wake::Changed);
			}

			Ok(_) => {}
			Err(error) => ui::warning(format_args!("could not watch for changes: {error}")),
		}
	})
	.context("could not start the file watcher")?;

	Ok((
		watcher,
		config_changed,
		tables_changed,
		Watched {
			metadata,
			public,
			excluded,
			config_file,
		},
	))
}

fn watch_roots(watcher: &mut notify::RecommendedWatcher, scope: &Watched) -> Result<()> {
	let mut recursive_roots = Vec::new();

	for package in &scope.metadata.packages {
		if let Some(dir) = package.manifest_path.parent() {
			watch_tree(watcher, dir, &scope.excluded, &mut recursive_roots)?;
		}
	}

	if scope.public.is_dir()
		&& !recursive_roots
			.iter()
			.any(|dir| scope.public.starts_with(dir))
	{
		watch_tree(
			watcher,
			&scope.public,
			&scope.excluded,
			&mut recursive_roots,
		)?;
	}

	for file in [
		scope.metadata.workspace_root.join("Cargo.toml"),
		scope.config_file.clone(),
	] {
		watcher
			.watch(&file, RecursiveMode::NonRecursive)
			.with_context(|| format!("could not watch {}", file.display()))?;
	}

	Ok(())
}

/// Rebuilds every platform after a debounced change. A config change warns
/// that the native apps themselves still need a restart, then reloads the
/// guests with the new configuration.
fn rebuild(
	mut started: Started,
	events_rx: &mpsc::Receiver<Wake>,
	config_changed: &AtomicBool,
	tables_changed: &AtomicBool,
) {
	while let Ok(Wake::Changed) = events_rx.recv() {
		// Debounce: keep absorbing events until none arrives for a whole
		// window, so an editor's burst of writes triggers one rebuild.
		loop {
			match events_rx.recv_timeout(REBUILD_DEBOUNCE) {
				Ok(Wake::Changed) => {}
				Ok(Wake::Stop) | Err(RecvTimeoutError::Disconnected) => return,
				Err(RecvTimeoutError::Timeout) => break,
			}
		}
		// Before the rebuild, so the server embeds a migration written now.
		if let Some(database) = &started.database
			&& tables_changed.swap(false, Ordering::Relaxed)
		{
			database.tables_changed();
		}

		if let Some(web) = &mut started.web {
			web.rebuild();
		}

		if let Some(extension) = &mut started.extension {
			extension.rebuild();
		}

		if let Some(native) = &started.native {
			reload_native(native, config_changed);
		}
	}
}

fn reload_native(native: &native::NativeDev, config_changed: &AtomicBool) {
	if config_changed.swap(false, Ordering::Relaxed) {
		ui::warning(
			"leptoskit.toml changed: guests rebuild with it, but restart `leptoskit dev` to \
			 rebuild the native apps themselves",
		);
	}

	match LeptoskitConfig::load() {
		Ok((config, _)) => native.reload(&config),
		Err(error) => {
			ui::warning(format_args!("could not reload leptoskit.toml: {error:#}"));
		}
	}
}

/// What wakes the dev loop.
enum Wake {
	/// A watched file changed.
	Changed,
	/// Ctrl-C or `SIGTERM` asked `dev` to stop.
	Stop,
}

/// The web half of `leptoskit dev`: the build in `dist/` and its server.
///
/// Which server depends on `app.render`. Client-rendered apps get
/// leptoskit's static server over `dist/`. Server-rendered apps (`ssr`,
/// `islands`) run their own server binary with the `LEPTOS_*` environment
/// pointing at `dist/`, restarted after every successful rebuild.
struct Web {
	config: LeptoskitConfig,
	address: SocketAddr,
	server_rendered: bool,
	app_server: Option<Child>,
	/// Extra variables for the app server, such as the database's URL.
	env: Vec<(String, String)>,
}

impl Web {
	/// Binds the configured address, builds, and starts serving.
	fn start(
		config: &LeptoskitConfig,
		root: &Path,
		port: Option<u16>,
		runtime: &tokio::runtime::Runtime,
		database: Option<&crate::db::DevDatabase>,
	) -> Result<Self> {
		let port = port.unwrap_or(config.serve.port);
		let ip: IpAddr = config.serve.address.parse().with_context(|| {
			format!(
				"invalid serve.address `{}`: expected an IP address such as 127.0.0.1",
				config.serve.address
			)
		})?;
		let address = SocketAddr::new(ip, port);
		// Bind first, so a taken port fails before the initial build rather
		// than leaving a dev loop with no server behind it.
		let listener = std::net::TcpListener::bind(address)
			.with_context(|| format!("could not bind {address}"))?;
		let server_rendered =
			config.app.render.renders_on_server() && config.app.default_platform == Platform::Web;
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
		crate::build::serve_plugins(config)?;
		let mut web = Self {
			config: config.clone(),
			address,
			server_rendered,
			app_server: None,
			env: database.map_or_else(Vec::new, crate::db::DevDatabase::env),
		};

		if let Some(listener) = listener {
			let out_dir = root.join(DEV_OUT);
			let router = leptoskit_axum::dev_router(&leptoskit_axum::ServeOptions::new(
				&out_dir,
				ip.to_string(),
				port,
			));
			let listener = {
				let _guard = runtime.enter();
				tokio::net::TcpListener::from_std(listener)
					.context("could not adopt the listener")?
			};
			runtime.spawn(async move {
				if let Err(error) = axum::serve(listener, router).await {
					ui::warning(format_args!("the dev server stopped: {error}"));
				}
			});
			ui::tagged(
				"web",
				format_args!(
					"serving {} at {BOLD}{}{BOLD:#}",
					out_dir.display(),
					web.url()
				),
			);
		} else {
			web.app_server = Some(start_app_server(config, &output, address, &web.env)?);
			ui::tagged(
				"web",
				format_args!("app server at {BOLD}{}{BOLD:#}", web.url()),
			);
		}

		Ok(web)
	}

	fn url(&self) -> String {
		format!("http://{}", self.address)
	}

	fn rebuild(&mut self) {
		// Stop the app server before cargo replaces its binary: Windows
		// refuses to overwrite a running executable.
		if let Some(mut child) = self.app_server.take() {
			stop(&mut child);
		}

		let started = Instant::now();

		match crate::build::run(false, None, None, DEV_OUT) {
			Ok(output) => {
				ui::tagged(
					"web",
					format_args!(
						"{GOOD}rebuilt{GOOD:#} {DIM}in {}{DIM:#}",
						ui::seconds(started.elapsed())
					),
				);
				if self.server_rendered {
					match start_app_server(&self.config, &output, self.address, &self.env) {
						Ok(child) => self.app_server = Some(child),
						Err(error) => {
							ui::tagged_error(
								"web",
								format_args!(
									"{BAD}could not restart the app server:{BAD:#} {error:#}"
								),
							);
						}
					}
				}
			}
			Err(error) => {
				ui::tagged_error("web", format_args!("{BAD}rebuild failed:{BAD:#} {error:#}"));
			}
		}
	}
}

impl Drop for Web {
	fn drop(&mut self) {
		if let Some(mut child) = self.app_server.take() {
			stop(&mut child);
		}
	}
}

/// Starts the server binary a build produced, in the leptos `DEV`
/// environment.
fn start_app_server(
	config: &LeptoskitConfig,
	output: &crate::build::BuildOutput,
	address: SocketAddr,
	env: &[(String, String)],
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
	crate::server::spawn_with(binary, &site, env)
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
	/// The generated routes module, when the file router is on.
	generated: Vec<PathBuf>,
}

impl WatchFilter {
	fn is_relevant(&self, path: &Path) -> bool {
		if self.excluded.iter().any(|dir| path.starts_with(dir))
			|| self
				.generated
				.iter()
				.any(|generated| path.starts_with(generated))
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

/// The directories the watcher ignores: build output, which changes on every
/// build, and leptoskit's own state.
fn excluded_dirs(
	root: &Path,
	metadata: &crate::wasm::CargoMetadata,
	out_dir: &Path,
) -> Vec<PathBuf> {
	let mut excluded = vec![
		metadata.target_directory.clone(),
		out_dir.to_path_buf(),
		root.join(".leptoskit"),
	];
	// Gradle keeps its caches and outputs inside the app-owned Android project.
	let android = crate::native::host_dir(root, crate::native::Target::Android);
	excluded.extend([".gradle", "build", "app/build", "app/.cxx"].map(|path| android.join(path)));
	excluded
}

/// Starts the app's development database, if it has one, and checks its
/// tables against the latest migration once.
fn start_database(config: &LeptoskitConfig, root: &Path) -> Result<Option<crate::db::DevDatabase>> {
	let database = crate::db::DevDatabase::start(config, root)?;

	if let Some(database) = &database {
		database.tables_changed();
	}

	Ok(database)
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
	fn generated_modules_never_trigger_a_rebuild() {
		let filter = filter(&["/app/src/routes.rs", "/app/src/gen/theme.rs"]);
		assert!(!filter.is_relevant(Path::new("/app/src/routes.rs")));
		assert!(!filter.is_relevant(Path::new("/app/src/gen/theme.rs")));
		assert!(filter.is_relevant(Path::new("/app/src/routes/index.rs")));
		assert!(filter.is_relevant(Path::new("/app/locales/en.arb")));
	}
}
