//! `leptoskit test extension`: the extension in a real browser.
//!
//! Builds the development package, loads it into Chrome for Testing through
//! chromedriver or Firefox through geckodriver, and checks what every extension
//! needs to work:
//!
//! - each page surface mounts the app and shows something, except the page the
//!   developer tools open out of sight, which only has to start it;
//! - the background worker is running;
//! - each content script puts its frame on a page whose content security policy
//!   forbids scripts and WebAssembly, as most large sites' do, and the frame
//!   mounts the app.

use std::net::SocketAddr;
use std::time::Duration;

use anyhow::Context;
use anyhow::Result;
use leptoskit_config::LeptoskitConfig;
use serde_json::json;

use super::browser::ExtensionBrowser;
use super::manifest::DevManifest;
use super::webdriver::Driver;

/// The page content scripts are tried on: a strict policy, like
/// github.com's.
const FIXTURE: &str = "<!doctype html><html><head><title>A strict page</title></head><body><h1>A \
                       strict page</h1><p id=\"text\">Heat rises. In cities, asphalt stores the \
                       day's heat.</p></body></html>";
const FIXTURE_POLICY: &str =
	"default-src 'self'; script-src 'self'; object-src 'none'; frame-src 'self'";

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
/// When the selected browser or its driver is missing, the build fails, or a
/// check fails.
pub fn run(browser: ExtensionBrowser) -> Result<()> {
	let (config, root) = LeptoskitConfig::load()?;
	let extension = config
		.extension
		.clone()
		.context("add an [extension] table to leptoskit.toml to test an extension")?;
	browser.support().map_err(anyhow::Error::msg)?;
	anyhow::ensure!(
		extension.browsers.contains(&browser.config()),
		"{browser:?} is not in extension.browsers"
	);
	let runtime = tokio::runtime::Runtime::new()?;
	let fixture = serve_fixture(&runtime)?;
	let built = super::build_for(
		false,
		Some(&DevManifest {
			key: None,
			server: fixture.trim_end_matches("/article").to_owned(),
		}),
		&[browser.config()],
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

	let driver = Driver::launch(browser, &dir, profile.path(), true)?;
	let base = driver.extension_base.clone();
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
		if !extension.has_page(&page, browser.config()) {
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
			.open(&format!("{base}/__leptoskit_probe.html"))
			.and_then(|()| driver.wait(background_condition(browser), Duration::from_secs(30))),
	);

	for script in &extension.content_scripts {
		if !extension.includes(script.browsers.as_deref(), browser.config()) {
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
				driver.open(&format!("{base}/__leptoskit_probe.html"))?;
				let injected = driver.script(&format!(
					"const done = arguments[arguments.length - 1]; (globalThis.browser ?? \
					 chrome).tabs.query({{ url: {url} }}).then(([tab]) => (globalThis.browser ?? \
					 chrome).scripting.executeScript({{ target: {{ tabId: tab.id }}, files: \
					 [{file}] }})).then(() => done(true), (error) => done(String(error)))",
					url = serde_json::to_string(&fixture)?,
					file = serde_json::to_string(&file)?,
				))?;
				anyhow::ensure!(
					injected == true,
					"content script injection failed: {injected}"
				);
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

fn background_condition(browser: ExtensionBrowser) -> &'static str {
	match browser {
		ExtensionBrowser::Chromium => {
			"navigator.serviceWorker.getRegistration().then(r => Boolean(r?.active))"
		}
		ExtensionBrowser::Firefox => {
			"browser.runtime.getBackgroundPage().then(page => \
			 Boolean(page?.__leptoskit_background_ready))"
		}
	}
}
