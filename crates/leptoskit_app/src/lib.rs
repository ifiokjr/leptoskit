//! <!-- {=appSummary|trim|linePrefix:"//! ":true} -->
//!
//! One `LeptosKit` component tree manages hosted installations from the web and
//! native iOS/Android clients. The accompanying `leptoskit-cloud` binary hosts
//! accounts, Stripe billing, per-account deployment journals, encrypted
//! `OpenTofu` state and the worker API. Customer code runs on a dedicated
//! outbound worker, using the existing CLI engine.
//!
//! <!-- {/appSummary} -->
pub mod client;
pub mod landing;
pub mod motion;
pub mod screen;
#[cfg(feature = "server")]
pub mod site;
use leptoskit_deploy_console::hosted_protocol::AccountView;
use reactive_graph::prelude::*;
use reactive_graph::signal::RwSignal;

pub const SERVER: &str = match option_env!("LEPTOSKIT_APP_ORIGIN") {
	Some(origin) => origin,
	None => "http://127.0.0.1:4701",
};

#[derive(Clone, Copy)]
pub struct Shell {
	pub key: RwSignal<String>,
	pub account: RwSignal<Option<AccountView>>,
	pub overview: RwSignal<serde_json::Value>,
	pub notice: RwSignal<String>,
	pub output: RwSignal<String>,
	pub measurement: RwSignal<serde_json::Value>,
	pub measured_project: RwSignal<String>,
	pub project: RwSignal<String>,
	pub version: RwSignal<String>,
	pub busy: RwSignal<bool>,
	pub setup: RwSignal<bool>,
	pub maintenance: RwSignal<bool>,
}
impl Default for Shell {
	fn default() -> Self {
		Self::new()
	}
}
impl Shell {
	pub fn new() -> Self {
		Self {
			key: RwSignal::new(String::new()),
			account: RwSignal::new(None),
			overview: RwSignal::new(serde_json::Value::Null),
			notice: RwSignal::new(String::new()),
			output: RwSignal::new(String::new()),
			measurement: RwSignal::new(serde_json::Value::Null),
			measured_project: RwSignal::new(String::new()),
			project: RwSignal::new(String::new()),
			version: RwSignal::new(String::new()),
			busy: RwSignal::new(false),
			setup: RwSignal::new(false),
			maintenance: RwSignal::new(false),
		}
	}

	pub fn request(self, path: String, body: Option<serde_json::Value>) {
		if self.busy.get_untracked() {
			return;
		}
		self.busy.set(true);
		self.output.set(String::new());
		self.notice.set("Working…".into());
		any_spawner::Executor::spawn_local(async move {
			let result = client::request(&self.key.get_untracked(), &path, body).await;
			match result {
				Ok(value) => {
					self.output.set(
						["code", "key", "url"]
							.into_iter()
							.find_map(|field| value[field].as_str())
							.unwrap_or_default()
							.to_owned(),
					);
					self.notice.set(if let Some(code) = value["code"].as_str() {
						format!("One-use pairing code (10 minutes): {code}")
					} else if let Some(key) = value["key"].as_str() {
						format!("Copy this key to your secret store now: {key}")
					} else if let Some(url) = value["url"].as_str() {
						format!("Open this secure billing link: {url}")
					} else if let Some(id) = value["id"].as_str() {
						format!("Operation {id} queued. Refresh to follow its status.")
					} else {
						"Saved.".into()
					});
					if path.ends_with("/analytics") {
						self.measurement.set(value.clone());
						self.measured_project.set(
							path.strip_prefix("/api/v1/projects/")
								.and_then(|s| s.strip_suffix("/analytics"))
								.unwrap_or_default()
								.to_owned(),
						);
					}
					if path == "/api/v1/account" {
						self.account.set(serde_json::from_value(value).ok());
					} else if path == "/api/v1/overview" {
						self.overview.set(value);
					}
					if self.account.get_untracked().is_some() {
						if let Ok(value) =
							client::request(&self.key.get_untracked(), "/api/v1/account", None)
								.await
						{
							self.account.set(serde_json::from_value(value).ok());
						}
						if let Ok(value) =
							client::request(&self.key.get_untracked(), "/api/v1/overview", None)
								.await
						{
							self.overview.set(value);
						}
					}
					#[cfg(all(target_arch = "wasm32", feature = "native"))]
					{
						let _ = client::save_key(&self.key.get_untracked()).await;
					}
				}
				Err(message) => self.notice.set(message),
			}
			self.busy.set(false);
		});
	}

	pub fn operation(self, project: &str, action: &serde_json::Value) {
		let revision = self.overview.get_untracked()["projects"]
			.as_array()
			.and_then(|projects| projects.iter().find(|p| p["id"] == project))
			.and_then(|p| p["revision"].as_u64());
		let Some(revision) = revision else {
			self.notice
				.set("Refresh application status before deploying.".into());
			return;
		};
		// The account revision prevents replay after another mutation; the
		// nonce identifies this intent.
		let nonce = std::sync::atomic::AtomicU64::fetch_add(
			&NONCE,
			1,
			std::sync::atomic::Ordering::Relaxed,
		);
		self.request(format!("/api/v1/projects/{project}/operations"),Some(serde_json::json!({"expected_revision":revision,"idempotency_key":format!("app-{revision}-{nonce}"),"action":action})));
	}

	pub fn logout(self) {
		self.key.set(String::new());
		self.output.set(String::new());
		self.notice.set(String::new());
		self.account.set(None);
		self.overview.set(serde_json::Value::Null);
		self.measurement.set(serde_json::Value::Null);
		self.measured_project.set(String::new());
		#[cfg(all(target_arch = "wasm32", feature = "native"))]
		any_spawner::Executor::spawn_local(async {
			let _ = client::save_key("").await;
		});
	}
}
static NONCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[cfg(all(target_arch = "wasm32", feature = "web"))]
#[leptos::component]
pub fn App() -> impl leptos::prelude::IntoView {
	motion::provide_runtime();
	let shell = Shell::new();
	move || leptoskit::web::render(screen::screen(shell))
}
#[cfg(all(target_arch = "wasm32", feature = "web"))]
#[leptos::component]
pub fn DemoApp() -> impl leptos::prelude::IntoView {
	motion::provide_runtime();
	let count = RwSignal::new(0);
	move || leptoskit::web::render(landing::demo(count))
}
#[cfg(all(target_arch = "wasm32", feature = "web"))]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
	use wasm_bindgen::JsCast as _;
	let target = web_sys::window()
		.and_then(|window| window.document())
		.and_then(|document| document.get_element_by_id("live-demo"))
		.and_then(|element| element.dyn_into::<web_sys::HtmlElement>().ok());
	if let Some(target) = target {
		target.set_text_content(None);
		leptos::mount::mount_to(target, DemoApp).forget();
	} else {
		leptos::mount::mount_to_body(App);
	}
}
#[cfg(all(target_arch = "wasm32", feature = "native"))]
mod guest;
