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

/// Starts the studio beside the app when asked to, over the database `dev`
/// uses.
fn start_studio(
	studio: bool,
	database: Option<&crate::db::DevDatabase>,
) -> Result<Option<crate::db::StudioProcess>> {
	if !studio {
		return Ok(None);
	}

	database
		.context("--studio serves the database, and leptoskit.toml has no `[db]` table")?
		.studio()
		.map(Some)
}

/// Runs the dev loop: an initial build of every selected platform, then a
/// watcher that rebuilds them once changes settle. `platforms` names the
/// platforms to run; empty means every enabled platform this machine can run.
/// `devices` narrows the mobile platforms to the simulators and devices named.
///
/// The terminal app needs this terminal to itself, so it runs alone: when it
/// is the one platform named, or when nothing is named and it is the app's
/// default platform.
#[derive(Debug, Clone, Copy)]
pub struct Options<'a> {
	/// Open the web app after it starts.
	pub open: bool,
	/// Override the configured web port.
	pub port: Option<u16>,
	/// Requested platforms, or empty for enabled platforms.
	pub platforms: &'a [Platform],
	/// Devices to run native apps on.
	pub devices: &'a [String],
	/// Disable the developer tools hub.
	pub no_devtools: bool,
	/// Override the developer tools port.
	pub devtools_port: Option<u16>,
	/// Serve the database studio beside the app.
	pub studio: bool,
	/// Browser used by the extension platform.
	pub browser: crate::extension::browser::ExtensionBrowser,
}

/// Starts the selected platforms and watches the app for changes.
/// # Errors
/// When a selected platform cannot start or the initial build fails.
pub fn run(options: Options<'_>) -> Result<()> {
	let Options {
		open,
		port,
		platforms,
		devices,
		no_devtools,
		devtools_port,
		studio,
		browser,
	} = options;
	let (config, root) = LeptoskitConfig::load()?;

	if terminal_only(&config, &root, platforms, open, port, devices, studio)? {
		return Ok(());
	}

	let mut tools = native::Tools::from_environment();
	tools.extension_browser = browser;
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
		studio,
	})?;
	let watching = watch(
		&config,
		&root,
		&runtime,
		open,
		started.web.as_ref(),
		started.database.as_ref(),
	)?;
	rebuild(started, &watching);
	Ok(())
}

fn terminal_only(
	config: &LeptoskitConfig,
	root: &Path,
	platforms: &[Platform],
	open: bool,
	port: Option<u16>,
	devices: &[String],
	studio: bool,
) -> Result<bool> {
	let terminal_only = match platforms {
		[] => config.app.default_platform == Platform::Tui,
		platforms => platforms.iter().all(|platform| *platform == Platform::Tui),
	};

	if !terminal_only {
		return Ok(false);
	}

	anyhow::ensure!(
		!open && port.is_none() && devices.is_empty() && !studio,
		"--open, --port, --device, and --studio serve other platforms; the terminal app runs in \
		 this terminal alone"
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
	/// The studio over the development database, stopped before it.
	_studio: Option<crate::db::StudioProcess>,
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
	studio: bool,
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
			studio,
		} = start;
		let devtools = if no_devtools {
			None
		} else {
			let hub = crate::devtools::start(
				root,
				runtime,
				devtools_port,
				port.unwrap_or(config.serve.port),
			)?;
			crate::build::devtools::set_hub(hub.url());
			Some(hub)
		};

		let database = start_database(config, root)?;
		let studio = start_studio(studio, database.as_ref())?;
		let web = selection
			.web
			.then(|| Web::start(config, root, port, runtime, database.as_ref()))
			.transpose()?;
		let extension = selection
			.extension
			.then(|| {
				crate::extension::dev::ExtensionDev::start(root, runtime, tools.extension_browser)
			})
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
			_studio: studio,
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
) -> Result<Watching> {
	let (events_tx, events_rx) = mpsc::channel::<Wake>();
	let stop = events_tx.clone();
	runtime.spawn(async move {
		leptoskit_axum::shutdown_signal().await;
		let _ = stop.send(Wake::Stop);
	});

	let (mut watcher, heard, scope) = start_watcher(config, root, events_tx, database)?;
	watch_roots(&mut watcher, &scope)?;
	heard.branch.watch(&mut watcher)?;

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

	Ok(Watching {
		_watcher: watcher,
		events: events_rx,
		heard,
	})
}

/// The running watcher, and what it has heard since the last rebuild.
struct Watching {
	/// Held for the whole loop: dropping it stops the watching.
	_watcher: notify::RecommendedWatcher,
	events: mpsc::Receiver<Wake>,
	heard: Heard,
}

/// What the watcher notes for the next rebuild, beyond that something changed.
struct Heard {
	config_changed: Arc<AtomicBool>,
	tables_changed: Arc<AtomicBool>,
	branch: BranchWatch,
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
) -> Result<(notify::RecommendedWatcher, Heard, Watched)> {
	let metadata = crate::wasm::cargo_metadata(root)?;
	let out_dir = root.join(DEV_OUT);
	let excluded = excluded_dirs(root, &metadata, &out_dir);
	let public = root.join("public");
	let config_file = root.join(leptoskit_config::CONFIG_FILE);
	let config_changed = Arc::new(AtomicBool::new(false));
	// The package that declares the tables: a change there may change them.
	let tables_dir = database.and_then(|database| database.package_dir(&metadata));
	let tables_changed = Arc::new(AtomicBool::new(false));
	let branch = BranchWatch::new(database.is_some(), root);

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
	let switched = branch.clone();
	let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
		match event {
			Ok(event) if switched.heard(&event) => drop(events_tx.send(Wake::Changed)),
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
		Heard {
			config_changed,
			tables_changed,
			branch,
		},
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
fn rebuild(mut started: Started, watching: &Watching) {
	let Watching { events, heard, .. } = watching;

	while let Ok(Wake::Changed) = events.recv() {
		// Debounce: keep absorbing events until none arrives for a whole
		// window, so an editor's burst of writes triggers one rebuild.
		loop {
			match events.recv_timeout(REBUILD_DEBOUNCE) {
				Ok(Wake::Changed) => {}
				Ok(Wake::Stop) | Err(RecvTimeoutError::Disconnected) => return,
				Err(RecvTimeoutError::Timeout) => break,
			}
		}

		// Before the rebuild, so the server starts on the branch's database
		// and embeds a migration written now.
		follow_database(
			started.database.as_mut(),
			&heard.branch,
			&heard.tables_changed,
			started.web.as_mut(),
		);

		if let Some(web) = &mut started.web {
			web.rebuild();
		}

		if let Some(extension) = &mut started.extension {
			extension.rebuild();
		}

		if let Some(native) = &started.native {
			reload_native(native, &heard.config_changed);
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

/// Git's `HEAD`, which it rewrites when the project switches branch: each
/// branch may have its own database.
#[derive(Clone)]
struct BranchWatch {
	head: Option<PathBuf>,
	switched: Arc<AtomicBool>,
}

impl BranchWatch {
	/// Follows the project's branch, when it has a database.
	fn new(database: bool, root: &Path) -> Self {
		Self {
			head: database
				.then(|| crate::db::branches::head_file(root))
				.flatten(),
			switched: Arc::new(AtomicBool::new(false)),
		}
	}

	/// Notes an event about `HEAD`, and says whether it was one.
	fn heard(&self, event: &notify::Event) -> bool {
		let heard = self
			.head
			.as_ref()
			.is_some_and(|head| event.paths.contains(head));

		if heard {
			self.switched.store(true, Ordering::Relaxed);
		}

		heard
	}

	/// Watches `HEAD`'s directory, since git replaces the file rather than
	/// writing it.
	fn watch(&self, watcher: &mut impl Watcher) -> Result<()> {
		if let Some(directory) = self.head.as_ref().and_then(|head| head.parent()) {
			watcher
				.watch(directory, RecursiveMode::NonRecursive)
				.with_context(|| format!("could not watch {}", directory.display()))?;
		}

		Ok(())
	}
}

/// Keeps the database in step with the project: the branch's own database
/// after a switch, which the app server restarts on with the next rebuild,
/// and the tables after they change.
fn follow_database(
	database: Option<&mut crate::db::DevDatabase>,
	branch: &BranchWatch,
	tables_changed: &AtomicBool,
	mut web: Option<&mut Web>,
) {
	let Some(database) = database else {
		return;
	};

	if branch.switched.swap(false, Ordering::Relaxed) {
		// Postgres copies a database only while nothing is connected to
		// it, and the server is connected to the branch it's leaving.
		if let Some(web) = web.as_deref_mut() {
			web.stop_app_server();
		}

		match database.switch_branch() {
			Ok(Some(env)) => {
				if let Some(web) = web {
					web.env = env;
					web.database_url = database.url().map(str::to_owned);
				}
			}
			Ok(None) => {}
			Err(error) => eprintln!("[db] couldn't switch to the branch's database: {error:#}"),
		}
	}

	if tables_changed.swap(false, Ordering::Relaxed) {
		database.tables_changed();
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
/// An app with a server package runs its own server binary, with the
/// `LEPTOS_*` environment pointing at `dist/`, restarted after every
/// successful rebuild: server-rendered apps (`ssr`, `islands`) need it to
/// render, and client-rendered ones to answer their API. A client-rendered
/// app without a server gets leptoskit's static server over `dist/`.
struct Web {
	config: LeptoskitConfig,
	root: PathBuf,
	/// Where browsers reach the app: `serve.port`.
	address: SocketAddr,
	/// Where the app (its server, or the static server) listens: `address`,
	/// or an internal port when the services' proxy holds `address`.
	app_address: SocketAddr,
	/// Whether the app's own server serves the site, so it restarts after
	/// each rebuild.
	runs_app_server: bool,
	app_server: Option<Child>,
	/// Extra variables for the app server, such as the database's URL.
	env: Vec<(String, String)>,
	/// The app's development database URL, beside which services get theirs.
	database_url: Option<String>,
	/// The app's services, running beside it.
	services: DevServices,
}

/// The services `leptoskit dev` runs, and what the app receives for them.
#[derive(Default)]
struct DevServices {
	/// Each service's token, kept across rebuilds.
	tokens: std::collections::BTreeMap<String, String>,
	running: crate::services::run::Running,
	/// The variables the app receives for the services.
	app_env: Vec<(String, String)>,
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

		let output = crate::build::run(false, None, None, DEV_OUT)?;
		crate::build::serve_plugins(config)?;
		// An app with a server package runs it, whatever its render mode: a
		// client-rendered app's server is where its API and sign-in live, and
		// it serves the built site itself. The app's server binds the port.
		let runs_app_server = server_rendered || output.server_binary.is_some();
		// Public services share the app's origin through a proxy on the
		// configured port, with the app behind it on an internal one.
		let routes = crate::services::proxy::Routes::of(&output.services);
		let app_address = if routes.is_empty() {
			address
		} else {
			free_address(ip)?
		};
		let listener = if runs_app_server && routes.is_empty() {
			drop(listener);
			None
		} else {
			listener
				.set_nonblocking(true)
				.context("could not configure the listener")?;
			Some(listener)
		};

		let mut web = Self {
			config: config.clone(),
			root: root.to_path_buf(),
			address,
			app_address,
			runs_app_server,
			app_server: None,
			env: database.map_or_else(Vec::new, crate::db::DevDatabase::env),
			database_url: database.and_then(|database| database.url().map(str::to_owned)),
			services: DevServices {
				tokens: crate::services::run::tokens(&output.services)?,
				..DevServices::default()
			},
		};
		web.start_services(&output.services)?;

		let adopt = |listener: std::net::TcpListener| {
			let _guard = runtime.enter();
			tokio::net::TcpListener::from_std(listener).context("could not adopt the listener")
		};

		let proxied = !routes.is_empty();

		if let Some(listener) = listener.as_ref().filter(|_| proxied) {
			let listener = listener
				.try_clone()
				.context("could not share the listener")?;
			let proxy = crate::services::proxy::router(routes, app_address);
			let listener = adopt(listener)?;
			runtime.spawn(async move {
				if let Err(error) = axum::serve(listener, proxy).await {
					ui::warning(format_args!("the services' proxy stopped: {error}"));
				}
			});
			ui::tagged(
				"services",
				format_args!("public paths proxied at {BOLD}{}{BOLD:#}", web.url()),
			);
		} else if let Some(listener) = listener {
			// No services to proxy, and no app server: leptoskit's static
			// server takes the configured port.
			serve_static(root, ip, port, adopt(listener)?, runtime);
			ui::tagged(
				"web",
				format_args!(
					"serving {} at {BOLD}{}{BOLD:#}",
					root.join(DEV_OUT).display(),
					web.url()
				),
			);
		}

		if runs_app_server {
			web.app_server = Some(start_app_server(
				config,
				&output,
				app_address,
				&web.app_env(),
			)?);
			ui::tagged(
				"web",
				format_args!("app server at {BOLD}{}{BOLD:#}", web.url()),
			);
		} else if app_address != address {
			let listener = std::net::TcpListener::bind(app_address)
				.with_context(|| format!("could not bind {app_address}"))?;
			listener
				.set_nonblocking(true)
				.context("could not configure the listener")?;
			serve_static(root, ip, app_address.port(), adopt(listener)?, runtime);
			ui::tagged(
				"web",
				format_args!(
					"serving {} at {BOLD}{}{BOLD:#}",
					root.join(DEV_OUT).display(),
					web.url()
				),
			);
		}

		Ok(web)
	}

	/// The app server's variables: the database's and the services'.
	fn app_env(&self) -> Vec<(String, String)> {
		self.env
			.iter()
			.chain(&self.services.app_env)
			.cloned()
			.collect()
	}

	/// Starts the services a build made, each ready before the next, with
	/// the tokens this session made for them.
	fn start_services(&mut self, built: &[crate::services::BuiltService]) -> Result<()> {
		if built.is_empty() {
			return Ok(());
		}

		// A service added since the session started gets a token too.
		for (name, token) in crate::services::run::tokens(built)? {
			self.services.tokens.entry(name).or_insert(token);
		}

		let session = crate::services::run::Session {
			config: &self.config,
			root: &self.root,
			public_origin: self.url(),
			app_url: format!("http://{}", self.app_address),
			app_database_url: self.database_url.clone(),
			tokens: &self.services.tokens,
		};
		let prepared = crate::services::run::prepare(built, &session)?;
		self.services.running = crate::services::run::start(
			&prepared,
			&[(crate::services::env::DRAIN_MS.to_owned(), "0".to_owned())],
			crate::server::DEV_STOP_GRACE,
		)?;
		self.services.app_env = prepared.app_env;
		Ok(())
	}

	fn url(&self) -> String {
		format!("http://{}", self.address)
	}

	/// Stops the app server, if it's running, until the next rebuild.
	fn stop_app_server(&mut self) {
		if let Some(mut child) = self.app_server.take() {
			stop(&mut child);
		}
	}

	fn rebuild(&mut self) {
		// Stop the app server and the services before cargo replaces their
		// binaries: Windows refuses to overwrite a running executable.
		self.stop_app_server();
		self.services.running.stop();
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

				if let Err(error) = self.start_services(&output.services) {
					ui::tagged_error(
						"services",
						format_args!("{BAD}could not restart the services:{BAD:#} {error:#}"),
					);
				}

				if self.runs_app_server {
					match start_app_server(&self.config, &output, self.app_address, &self.app_env())
					{
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

/// An address on `ip` with a port the OS says is free.
fn free_address(ip: IpAddr) -> Result<SocketAddr> {
	let listener = std::net::TcpListener::bind(SocketAddr::new(ip, 0))
		.context("could not find a free port for the app behind the services' proxy")?;
	listener
		.local_addr()
		.context("could not read the free port's address")
}

/// Serves `dist/` with leptoskit's static server on `listener`.
fn serve_static(
	root: &Path,
	ip: IpAddr,
	port: u16,
	listener: tokio::net::TcpListener,
	runtime: &tokio::runtime::Runtime,
) {
	let out_dir = root.join(DEV_OUT);
	let router = leptoskit_axum::dev_router(&leptoskit_axum::ServeOptions::new(
		&out_dir,
		ip.to_string(),
		port,
	));
	runtime.spawn(async move {
		if let Err(error) = axum::serve(listener, router).await {
			ui::warning(format_args!("the dev server stopped: {error}"));
		}
	});
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
	crate::server::ensure_port_free(address, "the app server")?;
	crate::server::spawn_with(binary, &site, env)
}

/// Stops a child process (`SIGTERM`, then `SIGKILL` after
/// [`DEV_STOP_GRACE`](crate::server::DEV_STOP_GRACE)) and reaps it. A child
/// that already exited is fine.
fn stop(child: &mut Child) {
	crate::server::stop_gracefully(&mut [child], crate::server::DEV_STOP_GRACE);
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

	#[test]
	fn the_studio_starts_only_when_asked_and_needs_a_database() {
		assert!(start_studio(false, None).unwrap().is_none());
		let error = start_studio(true, None).err().unwrap();
		assert!(
			error.to_string().contains("--studio serves the database"),
			"{error}"
		);
	}

	#[test]
	fn the_dev_loop_follows_git_s_head_and_the_tables() {
		let root = tempfile::tempdir().unwrap();
		assert!(
			BranchWatch::new(true, root.path()).head.is_none(),
			"not a repository"
		);
		let status = crate::git::command(root.path())
			.args(["init", "--quiet"])
			.status()
			.unwrap();
		assert!(status.success());
		assert!(
			BranchWatch::new(false, root.path()).head.is_none(),
			"no database"
		);

		let branch = BranchWatch::new(true, root.path());
		let head = branch.head.clone().unwrap();
		let event =
			|path: &Path| notify::Event::new(notify::EventKind::Any).add_path(path.to_path_buf());
		assert!(!branch.heard(&event(&root.path().join("src/lib.rs"))));
		assert!(branch.heard(&event(&head)));
		let mut watcher = notify::recommended_watcher(|_| {}).unwrap();
		branch.watch(&mut watcher).unwrap();

		// Without a database there's nothing to follow; with one, a switch
		// outside the development Postgres changes nothing, and changed
		// tables are read again, which fails here and carries on.
		let tables = AtomicBool::new(true);
		follow_database(None, &branch, &tables, None);
		assert!(tables.load(Ordering::Relaxed));

		let config: LeptoskitConfig =
			toml::from_str("[app]\nname = \"app\"\n[db]\ndialect = \"sqlite\"\nwatch = \"push\"\n")
				.unwrap();
		let mut database = crate::db::DevDatabase::start(&config, root.path())
			.unwrap()
			.unwrap();
		follow_database(Some(&mut database), &branch, &tables, None);
		assert!(!branch.switched.load(Ordering::Relaxed));
		assert!(!tables.load(Ordering::Relaxed));
	}

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
