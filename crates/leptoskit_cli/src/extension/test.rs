//! `leptoskit test extension`: the extension in a real browser.
//!
//! Builds the development package, loads it into Chrome for Testing through
//! chromedriver, and checks what every extension needs to work:
//!
//! - each page surface mounts the app and shows something, except the page the
//!   developer tools open out of sight, which only has to start it;
//! - the background worker is running;
//! - each content script puts its frame on a page whose content security policy
//!   forbids scripts and WebAssembly, as most large sites' do, and the frame
//!   mounts the app.

use std::net::SocketAddr;
use std::path::Path;
use std::path::PathBuf;
use std::process::Child;
use std::process::Command;
use std::process::Stdio;
use std::time::Duration;
use std::time::Instant;

use anyhow::Context;
use anyhow::Result;
use leptoskit_config::Browser;
use leptoskit_config::LeptoskitConfig;
use serde_json::Value;
use serde_json::json;

use super::dev::unpacked_id;
use super::manifest::DevManifest;

/// The environment variable that names a chromedriver executable.
pub const CHROMEDRIVER_ENV: &str = "LEPTOSKIT_CHROMEDRIVER";

/// The page content scripts are tried on: a strict policy, like
/// github.com's.
const FIXTURE: &str = "<!doctype html><html><head><title>A strict page</title></head><body><h1>A \
                       strict page</h1><p id=\"text\">Heat rises. In cities, asphalt stores the \
                       day's heat.</p></body></html>";
const FIXTURE_POLICY: &str =
	"default-src 'self'; script-src 'self'; object-src 'none'; frame-src 'self'";

/// chromedriver: `LEPTOSKIT_CHROMEDRIVER`, the newest one in the Chrome for
/// Testing installer's cache, or `chromedriver` on the `PATH`.
fn chromedriver() -> PathBuf {
	if let Some(path) = std::env::var_os(CHROMEDRIVER_ENV) {
		return PathBuf::from(path);
	}
	let cached = std::env::var_os("HOME")
		.or_else(|| std::env::var_os("USERPROFILE"))
		.map(|home| PathBuf::from(home).join(".cache/puppeteer/chromedriver"))
		.and_then(|cache| std::fs::read_dir(cache).ok())
		.and_then(|installs| {
			let mut installs: Vec<PathBuf> = installs
				.filter_map(Result::ok)
				.map(|entry| entry.path())
				.collect();
			installs.sort();
			installs.into_iter().rev().find_map(|install| {
				[
					"chromedriver-mac-arm64",
					"chromedriver-mac-x64",
					"chromedriver-linux64",
					"chromedriver-win64",
				]
				.into_iter()
				.map(|dir| {
					install.join(dir).join(if cfg!(windows) {
						"chromedriver.exe"
					} else {
						"chromedriver"
					})
				})
				.find(|path| path.is_file())
			})
		});
	cached.unwrap_or_else(|| PathBuf::from("chromedriver"))
}

/// One W3C `WebDriver` session.
struct Driver {
	process: Child,
	base: String,
	session: String,
	agent: ureq::Agent,
}

impl Driver {
	fn start(chrome: &Path, extension: &Path, profile: &Path) -> Result<Self> {
		let port = {
			let listener = std::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))?;
			listener.local_addr()?.port()
		};
		let driver = chromedriver();
		let process = Command::new(&driver)
			.arg(format!("--port={port}"))
			.stdout(Stdio::null())
			.stderr(Stdio::null())
			.spawn()
			.with_context(|| {
				format!(
					"could not start {}: install chromedriver for your Chrome for Testing with \
					 `npx @puppeteer/browsers install chromedriver@stable --path \
					 ~/.cache/puppeteer`, or set {CHROMEDRIVER_ENV}",
					driver.display()
				)
			})?;
		let agent: ureq::Agent = ureq::Agent::config_builder()
			.timeout_global(Some(Duration::from_mins(1)))
			.http_status_as_error(false)
			.build()
			.into();
		let base = format!("http://127.0.0.1:{port}");
		let mut driver = Self {
			process,
			base,
			session: String::new(),
			agent,
		};
		let deadline = Instant::now() + Duration::from_secs(20);
		loop {
			if driver.call("GET", "/status", None).is_ok() {
				break;
			}
			anyhow::ensure!(Instant::now() < deadline, "chromedriver didn't start");
			std::thread::sleep(Duration::from_millis(200));
		}
		let session = driver.call(
			"POST",
			"/session",
			Some(json!({
				"capabilities": { "alwaysMatch": { "browserName": "chrome", "goog:chromeOptions": {
					"binary": chrome,
					"args": [
						format!("--load-extension={}", extension.display()),
						format!("--disable-extensions-except={}", extension.display()),
						format!("--user-data-dir={}", profile.display()),
						"--headless=new",
						"--no-first-run",
						"--no-default-browser-check",
					],
				}}}
			})),
		)?;
		session["sessionId"]
			.as_str()
			.context("chromedriver started no session")?
			.clone_into(&mut driver.session);
		Ok(driver)
	}

	fn call(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value> {
		let url = format!("{}{path}", self.base);
		let mut response = match (method, body) {
			("GET", _) => self.agent.get(&url).call()?,
			("DELETE", _) => self.agent.delete(&url).call()?,
			(_, body) => {
				self.agent
					.post(&url)
					.header("content-type", "application/json")
					.send(serde_json::to_vec(&body.unwrap_or_else(|| json!({})))?)?
			}
		};
		let value: Value = serde_json::from_str(&response.body_mut().read_to_string()?)?;
		if let Some(error) = value["value"]["error"].as_str() {
			anyhow::bail!(
				"{error}: {}",
				value["value"]["message"].as_str().unwrap_or_default()
			);
		}
		Ok(value["value"].clone())
	}

	fn session(&self, method: &str, tail: &str, body: Option<Value>) -> Result<Value> {
		self.call(method, &format!("/session/{}{tail}", self.session), body)
	}

	fn open(&self, url: &str) -> Result<()> {
		self.session("POST", "/url", Some(json!({ "url": url })))
			.map(drop)
	}

	fn script(&self, script: &str) -> Result<Value> {
		self.session(
			"POST",
			"/execute/async",
			Some(json!({ "script": script, "args": [] })),
		)
	}

	/// Waits until `condition`, a JavaScript expression, is truthy.
	fn wait(&self, condition: &str, timeout: Duration) -> Result<()> {
		let deadline = Instant::now() + timeout;
		let script = format!(
			"const done = arguments[arguments.length - 1]; Promise.resolve().then(() => \
			 ({condition})).then((value) => done(Boolean(value)), () => done(false));"
		);
		loop {
			if self.script(&script).ok().and_then(|value| value.as_bool()) == Some(true) {
				return Ok(());
			}
			anyhow::ensure!(
				Instant::now() < deadline,
				"timed out waiting for {condition}"
			);
			std::thread::sleep(Duration::from_millis(250));
		}
	}
}

impl Drop for Driver {
	fn drop(&mut self) {
		if !self.session.is_empty() {
			let _ = self.call("DELETE", &format!("/session/{}", self.session), None);
		}
		let _ = self.process.kill();
		let _ = self.process.wait();
	}
}

/// Serves the strict fixture page on loopback, and returns its address.
fn serve_fixture(runtime: &tokio::runtime::Runtime) -> Result<String> {
	let listener = std::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))?;
	listener.set_nonblocking(true)?;
	let address = listener.local_addr()?;
	let listener = {
		let _guard = runtime.enter();
		tokio::net::TcpListener::from_std(listener)?
	};
	let router = axum::Router::new().fallback(|| {
		async {
			(
				[
					(axum::http::header::CONTENT_TYPE, "text/html"),
					(axum::http::header::CONTENT_SECURITY_POLICY, FIXTURE_POLICY),
				],
				FIXTURE,
			)
		}
	});
	runtime.spawn(async move {
		let _ = axum::serve(listener, router).await;
	});
	Ok(format!("http://{address}/article"))
}

/// `leptoskit test extension`.
///
/// # Errors
///
/// When Chrome for Testing or chromedriver is missing, the build fails, or a
/// check fails.
pub fn run() -> Result<()> {
	let (config, root) = LeptoskitConfig::load()?;
	let extension = config
		.extension
		.clone()
		.context("add an [extension] table to leptoskit.toml to test an extension")?;
	let chrome = super::dev::chrome().context(super::dev::support().err().unwrap_or_default())?;
	let runtime = tokio::runtime::Runtime::new()?;
	let fixture = serve_fixture(&runtime)?;
	let built = super::build(
		false,
		Some(&DevManifest {
			key: None,
			server: fixture.trim_end_matches("/article").to_owned(),
		}),
	)?;
	let dir = built
		.packages
		.first()
		.map(|package| {
			package
				.dir
				.canonicalize()
				.unwrap_or_else(|_| package.dir.clone())
		})
		.context("the build made no package")?;
	let profile = tempfile::tempdir()?;
	let driver = Driver::start(&chrome, &dir, profile.path())?;
	let id = unpacked_id(&dir);
	let base = format!("chrome-extension://{id}");
	let mut failures = Vec::new();
	let mut check = |name: String, result: Result<()>| {
		match result {
			Ok(()) => println!("ok    {name}"),
			Err(error) => {
				println!("FAIL  {name}: {error:#}");
				failures.push(name);
			}
		}
	};

	for page in extension.pages() {
		if !extension.has_page(&page, Browser::Chromium) {
			continue;
		}
		// The developer tools open `devtools.html` out of sight to add the
		// panel, so it shows nothing: it only has to start the app.
		let (name, shown) = if page.name == "devtools" {
			(
				format!("the {} page starts at {}", page.name, page.route),
				"document.body.children.length > 0",
			)
		} else {
			(
				format!("the {} shows {}", page.name, page.route),
				"document.body.children.length > 0 && document.body.innerText.trim().length > 0",
			)
		};
		check(
			name,
			driver
				.open(&format!("{base}/{}.html", page.name))
				.and_then(|()| driver.wait(shown, Duration::from_secs(30))),
		);
	}

	// Any extension page can see the worker's registration.
	check(
		"the background worker runs".to_owned(),
		driver
			.open(&format!("{base}/manifest.json"))
			.and_then(|()| {
				driver.wait(
					"navigator.serviceWorker.getRegistration().then((registration) => \
					 Boolean(registration?.active))",
					Duration::from_secs(30),
				)
			}),
	);

	for script in &extension.content_scripts {
		if !extension.includes(script.browsers.as_deref(), Browser::Chromium) {
			continue;
		}
		let file = format!("content-{}.js", script.name);
		let result = driver
			.open(&fixture)
			.and_then(|()| driver.session("GET", "/window", None))
			.and_then(|page| {
				let page = page.as_str().context("no window handle")?.to_owned();
				// Inject from an extension page in another tab, as the app's
				// own code does after the user invokes it.
				let tab = driver.session("POST", "/window/new", Some(json!({ "type": "tab" })))?;
				let tab = tab["handle"].as_str().context("no new tab")?.to_owned();
				driver.session("POST", "/window", Some(json!({ "handle": tab })))?;
				driver.open(&format!("{base}/manifest.json"))?;
				driver.script(&format!(
					"const done = arguments[arguments.length - 1]; chrome.tabs.query({{ url: \
					 {url} }}).then(([tab]) => chrome.scripting.executeScript({{ target: {{ \
					 tabId: tab.id }}, files: [{file}] }})).then(() => done(true), (error) => \
					 done(String(error)))",
					url = serde_json::to_string(&fixture)?,
					file = serde_json::to_string(&file)?,
				))?;
				driver.session("DELETE", "/window", None)?;
				driver.session("POST", "/window", Some(json!({ "handle": page })))?;
				driver.wait(
					"document.querySelector('leptoskit-frame')",
					Duration::from_secs(15),
				)
			});
		check(
			format!(
				"the content script `{}` puts its frame on a strict page",
				script.name
			),
			result,
		);
	}

	drop(driver);
	drop(runtime);
	anyhow::ensure!(
		failures.is_empty(),
		"{} of the extension's checks failed in {}",
		failures.len(),
		root.display()
	);
	println!("every check passed");
	Ok(())
}
