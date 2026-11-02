//! Owned browser sessions shared by development and extension smoke tests.
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
use serde_json::Value;
use serde_json::json;

use super::browser::ExtensionBrowser;
/// Explicit chromedriver executable.
pub const CHROMEDRIVER_ENV: &str = "LEPTOSKIT_CHROMEDRIVER";
/// chromedriver: `LEPTOSKIT_CHROMEDRIVER`, the newest one in the Chrome for
/// Testing installer's cache, or `chromedriver` on the `PATH`.
fn chromedriver() -> PathBuf {
	if let Some(path) = std::env::var_os(CHROMEDRIVER_ENV) {
		return PathBuf::from(path);
	}

	cached_chromedriver().unwrap_or_else(|| PathBuf::from("chromedriver"))
}

fn cached_chromedriver() -> Option<PathBuf> {
	let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
	let entries = std::fs::read_dir(Path::new(&home).join(".cache/puppeteer/chromedriver")).ok()?;
	let mut installs: Vec<PathBuf> = entries
		.filter_map(Result::ok)
		.map(|entry| entry.path())
		.collect();
	installs.sort();
	installs
		.into_iter()
		.rev()
		.find_map(|install| chromedriver_in(&install))
}

fn chromedriver_in(install: &Path) -> Option<PathBuf> {
	let executable = if cfg!(windows) {
		"chromedriver.exe"
	} else {
		"chromedriver"
	};
	[
		"chromedriver-mac-arm64",
		"chromedriver-mac-x64",
		"chromedriver-linux64",
		"chromedriver-win64",
	]
	.into_iter()
	.map(|dir| install.join(dir).join(executable))
	.find(|path| path.is_file())
}

/// One W3C `WebDriver` session.
pub(crate) struct Driver {
	pub(crate) extension_base: String,
	bidi: Option<super::bidi::Connection>,
	addon: Option<String>,
	process: Option<Child>,
	isolated: bool,
	base: String,
	session: String,
	agent: ureq::Agent,
}

impl Driver {
	fn chrome(chrome: &Path, extension: &Path, profile: &Path) -> Result<Self> {
		let mut driver = Self::spawn(&chromedriver(), true)?;
		let session = driver.call(
			"POST",
			"/session",
			Some(json!({
				"capabilities": { "alwaysMatch": { "browserName": "chrome", "timeouts": {"script": 10000, "pageLoad": 30000}, "goog:chromeOptions": {
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

	fn spawn(program: &Path, isolated: bool) -> Result<Self> {
		let port = {
			let listener = std::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))?;
			listener.local_addr()?.port()
		};
		let mut command = Command::new(program);
		// Headless smoke sessions own a process group so a failed startup
		// can terminate the whole tree. Interactive sessions stay in the
		// terminal's group so Ctrl-C also reaches geckodriver and Firefox.
		if isolated {
			crate::testing::isolate(&mut command);
		}
		let process = command
			.arg(format!("--port={port}"))
			.stdout(Stdio::null())
			.stderr(Stdio::null())
			.spawn()
			.with_context(|| {
				format!(
					"could not start {}; check leptoskit doctor",
					program.display()
				)
			})?;
		let agent: ureq::Agent = ureq::Agent::config_builder()
			.timeout_global(Some(Duration::from_secs(30)))
			.http_status_as_error(false)
			.build()
			.into();
		let base = format!("http://127.0.0.1:{port}");
		let driver = Self {
			process: Some(process),
			isolated,
			base,
			session: String::new(),
			extension_base: String::new(),
			bidi: None,
			addon: None,
			agent,
		};
		let deadline = Instant::now() + Duration::from_secs(20);

		loop {
			if driver.call("GET", "/status", None).is_ok() {
				break;
			}

			anyhow::ensure!(
				Instant::now() < deadline,
				"WebDriver did not start within 20 seconds"
			);
			std::thread::sleep(Duration::from_millis(200));
		}

		Ok(driver)
	}

	pub(crate) fn call(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value> {
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

	pub(crate) fn session(&self, method: &str, tail: &str, body: Option<Value>) -> Result<Value> {
		self.call(method, &format!("/session/{}{tail}", self.session), body)
	}

	pub(crate) fn open(&self, url: &str) -> Result<()> {
		self.session("POST", "/url", Some(json!({ "url": url })))
			.map(drop)
	}

	pub(crate) fn script(&self, script: &str) -> Result<Value> {
		self.session(
			"POST",
			"/execute/async",
			Some(json!({ "script": script, "args": [] })),
		)
	}

	/// Waits until `condition`, a JavaScript expression, is truthy.
	pub(crate) fn wait(&self, condition: &str, timeout: Duration) -> Result<()> {
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

		let Some(process) = &mut self.process else {
			return;
		};
		if self.isolated {
			let _ = crate::testing::terminate(process);
		} else {
			let _ = process.kill();
		}
		let deadline = Instant::now() + Duration::from_secs(2);

		while matches!(process.try_wait(), Ok(None)) && Instant::now() < deadline {
			std::thread::sleep(Duration::from_millis(20));
		}
	}
}

impl Driver {
	pub(crate) fn launch(
		browser: ExtensionBrowser,
		extension: &Path,
		profile: &Path,
		headless: bool,
	) -> Result<Self> {
		let extension = extension
			.canonicalize()
			.context("extension package directory is missing")?;
		match browser {
			ExtensionBrowser::Chromium => {
				let chrome = super::dev::chrome().context("Chrome for Testing is missing")?;
				let mut driver = Self::chrome(&chrome, &extension, profile)?;
				driver.extension_base =
					format!("chrome-extension://{}", super::dev::unpacked_id(&extension));
				Ok(driver)
			}

			ExtensionBrowser::Firefox => Self::firefox(&extension, headless),
		}
	}

	fn firefox(extension: &Path, headless: bool) -> Result<Self> {
		let binary =
			super::browser::firefox().context("Firefox is missing; set LEPTOSKIT_FIREFOX")?;
		let manifest: Value =
			serde_json::from_slice(&std::fs::read(extension.join("manifest.json"))?)?;
		let addon = manifest["browser_specific_settings"]["gecko"]["id"]
			.as_str()
			.context("Firefox needs extension.firefox.id")?;
		let uuid = uuid::Uuid::new_v4().to_string();
		let mut driver = Self::spawn(
			&super::browser::geckodriver()
				.context("geckodriver is missing; set LEPTOSKIT_GECKODRIVER")?,
			headless,
		)?;
		driver.start_firefox(extension, &binary, addon, &uuid, headless)?;
		Ok(driver)
	}

	fn start_firefox(
		&mut self,
		extension: &Path,
		binary: &Path,
		addon: &str,
		uuid: &str,
		headless: bool,
	) -> Result<()> {
		let args = if headless { vec!["-headless"] } else { vec![] };
		// Firefox normally deletes the UUID and storage on uninstall. Keeping
		// both inside this fresh profile gives rebuilds the same origin and
		// persisted extension data without touching the user's own profile.
		let prefs = json!({
			"extensions.webextensions.uuids": serde_json::to_string(&json!({addon: uuid}))?,
			"extensions.webextensions.keepUuidOnUninstall": true,
			"extensions.webextensions.keepStorageOnUninstall": true,
		});
		let result = self.call(
			"POST",
			"/session",
			Some(json!({
				"capabilities": {"alwaysMatch": {
					"browserName": "firefox", "webSocketUrl": true,
					"timeouts": {"script": 10000, "pageLoad": 30000},
					"moz:firefoxOptions": {
						"binary": binary,
						"args": args,
						"prefs": prefs
					}
				}}
			})),
		)?;
		result["sessionId"]
			.as_str()
			.context("geckodriver returned no session ID")?
			.clone_into(&mut self.session);
		let socket = result["capabilities"]["webSocketUrl"]
			.as_str()
			.context("Firefox returned no BiDi socket")?;
		self.bidi = Some(super::bidi::Connection::connect(socket)?);
		self.install(extension)?;
		self.extension_base = format!("moz-extension://{uuid}");
		Ok(())
	}

	fn install(&mut self, extension: &Path) -> Result<()> {
		let bidi = self.bidi.as_mut().context("no BiDi session")?;
		let result = bidi.call(
			"webExtension.install",
			&json!({"extensionData": {"type": "path", "path": extension}}),
		)?;
		self.addon = Some(
			result["extension"]
				.as_str()
				.context("Firefox returned no extension ID")?
				.to_owned(),
		);
		Ok(())
	}

	pub(crate) fn reinstall(&mut self, extension: &Path) -> Result<()> {
		if let Some(addon) = self.addon.as_ref() {
			self.bidi
				.as_mut()
				.context("no BiDi session")?
				.call("webExtension.uninstall", &json!({"extension": addon}))?;
			self.addon = None;
		}

		self.install(extension)
	}
}

#[cfg(test)]
#[path = "webdriver_tests.rs"]
mod tests;
