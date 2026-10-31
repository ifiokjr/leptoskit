//! Every generated `--idp` screen and route renders headlessly. With no
//! server to reach, every change answers the client's error (`config`
//! alone, or `server` when another crate turns on server rendering), so the
//! screens show it as an alert; reads are seeded into the cache the way a
//! server render would have left them.

use std::future::Future;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use auth_pages_example::auth;
use auth_pages_example::auth::Screens;
use auth_pages_example::route_connected_apps;
use auth_pages_example::route_consent;
use auth_pages_example::route_developer_apps;
use auth_pages_example::route_device;
use auth_pages_example::route_devices;
use auth_pages_example::route_security;
use auth_pages_example::route_sign_in;
use auth_pages_example::route_sign_in_idp;
use leptoskit::auth::PublicSession;
use leptoskit::idp::client;
use leptoskit::prelude::Element;
use leptoskit::prelude::Text;
use leptoskit::query::NetworkMode;
use leptoskit::query::QueryClient;
use leptoskit::query::QueryDefaults;
use leptoskit::query::RefetchOn;
use leptoskit::query::RetryPolicy;
use leptoskit::query::StaleTime;
use leptoskit::view;
use leptoskit_testing::Finder;
use leptoskit_testing::WidgetTester;
use reactive_graph::owner::Owner;

/// How an error shows in the semantics tree.
const ALERT: &str = "alert \"";

/// A screen under test: its tester, the screens, the cache, and where the
/// screens tried to send the browser.
struct Shown {
	tester: WidgetTester,
	screens: Screens,
	cache: QueryClient,
	visited: Arc<Mutex<Vec<String>>>,
}

impl Shown {
	fn text(&self) -> String {
		self.tester.semantics()
	}

	fn has(&self, text: &str) -> bool {
		self.text().contains(text)
	}

	async fn tap(&mut self, finder: &Finder) {
		self.tester.tap(finder).unwrap();
		self.settle().await;
	}

	async fn settle(&mut self) {
		// A few rounds at least: a key set while drawing rebinds its query in
		// an effect, which then fetches.
		for round in 0..50 {
			any_spawner::Executor::tick().await;
			tokio::time::sleep(Duration::from_millis(2)).await;
			self.tester.pump();
			if round >= 5 && self.cache.fetching_count() == 0 && self.cache.mutating_count() == 0 {
				break;
			}
		}
	}
}

/// Runs a test where the screens' effects can run: on this thread, boxed,
/// since a test's future holds whole screens.
async fn local<F: Future<Output = ()>>(test: impl FnOnce() -> F) {
	tokio::task::LocalSet::new()
		.run_until(Box::pin(test()))
		.await;
}

/// A session, as the session query holds it once someone signs in.
fn session() -> PublicSession {
	let now = chrono::Utc::now();
	serde_json::from_value(serde_json::json!({
		"id": "00000000-0000-4000-8000-000000000001",
		"account_id": "00000000-0000-4000-8000-000000000002",
		"aal": "aal1",
		"authenticated_at": now,
		"mfa_at": null,
		"last_seen_at": now,
		"idle_expires_at": now,
		"expires_at": now,
		"device": "This laptop",
		"revoked_at": null,
		"created_at": now,
	}))
	.unwrap()
}

/// Provides the screens in their own owner, over a cache `seed` filled, and
/// draws what `draw` picks once the reads have settled. `signed_in` seeds
/// the session query: `Some(true)` signed in, `Some(false)` signed out, and
/// `None` not known yet.
async fn show(
	signed_in: Option<bool>,
	seed: impl FnOnce(&QueryClient),
	navigate: bool,
	draw: impl Fn(Screens) -> Element + Send + Sync + 'static,
) -> Shown {
	let _ = any_spawner::Executor::init_tokio();
	let owner = Owner::new();
	let visited = Arc::new(Mutex::new(Vec::new()));
	let log = Arc::clone(&visited);
	let (screens, cache) = owner.with(|| {
		let cache = QueryClient::new().with_defaults(
			QueryDefaults::new()
				.with_retry(RetryPolicy::Never)
				.with_refetch_on_window_focus(RefetchOn::Never)
				.with_refetch_on_reconnect(RefetchOn::Never)
				.with_network_mode(NetworkMode::Always)
				.with_stale_time(StaleTime::Static),
		);
		cache.provide();

		if let Some(signed_in) = signed_in {
			cache.set_query_data(
				&leptoskit::auth::client::session(),
				(),
				signed_in.then(session),
			);
		}

		seed(&cache);
		let screens = auth::provide();

		if navigate {
			screens.navigate_with(move |to| log.lock().unwrap().push(to.to_owned()));
		}

		(screens, cache)
	});
	let drawing = owner.clone();
	let tester = WidgetTester::new(move || drawing.with(|| draw(screens)), 800.0, 800.0).unwrap();
	let mut shown = Shown {
		tester,
		screens,
		cache,
		visited,
	};
	shown.settle().await;
	shown
}

fn scopes() -> Vec<leptoskit::idp::ScopeDescription> {
	vec![
		leptoskit::idp::ScopeDescription {
			name: "openid".into(),
			description: "Sign you in with your account.".into(),
		},
		leptoskit::idp::ScopeDescription {
			name: "notes:read".into(),
			description: String::new(),
		},
	]
}

fn partner() -> leptoskit::idp::AppInfo {
	leptoskit::idp::AppInfo {
		client_id: "partner".into(),
		name: "Partner".into(),
	}
}

fn seed_consent(cache: &QueryClient) {
	cache.set_query_data(
		&client::consent(),
		"handle".into(),
		leptoskit::idp::ConsentRequest {
			app: partner(),
			scopes: scopes(),
			redirect_host: "partner.example".into(),
		},
	);
}

#[tokio::test]
async fn the_consent_screen_shows_the_app_and_takes_an_answer() {
	local(|| {
		async {
			let mut shown = show(Some(true), seed_consent, true, |_| {
				route_consent::page(&route_consent::Consent {
					request: Some("handle".into()),
				})
			})
			.await;

			assert!(
				shown.has("Partner wants to use your account"),
				"{}",
				shown.text()
			);
			assert!(shown.has("Sign you in with your account."));
			assert!(shown.has("• notes:read"), "{}", shown.text());
			assert!(shown.has("partner.example"));

			shown.tap(&Finder::text("Allow")).await;
			assert!(shown.has(ALERT), "{}", shown.text());
			shown.tap(&Finder::text("Cancel")).await;
			assert!(shown.has(ALERT), "{}", shown.text());
			// Nothing came back from a server, so nothing navigated.
			assert!(shown.visited.lock().unwrap().is_empty());
		}
	})
	.await;
}

#[tokio::test]
async fn the_consent_screen_needs_a_request_and_a_signed_in_person() {
	local(|| {
		async {
			let shown = show(Some(true), |_| {}, true, |screens| screens.consent.screen()).await;
			assert!(shown.has("needs the request"), "{}", shown.text());

			let shown = show(
				Some(true),
				|_| {},
				true,
				|_| {
					route_consent::page(&route_consent::Consent {
						request: Some("unknown".into()),
					})
				},
			)
			.await;
			assert!(shown.has(ALERT), "{}", shown.text());
			assert!(shown.has("Try again"), "{}", shown.text());

			let shown = show(Some(false), seed_consent, true, |_| {
				route_consent::page(&route_consent::Consent {
					request: Some("handle".into()),
				})
			})
			.await;
			assert!(shown.has("Email address"), "{}", shown.text());
			assert!(!shown.has("Partner"), "{}", shown.text());
		}
	})
	.await;
}

#[tokio::test]
async fn the_device_screen_takes_a_typed_code_or_one_from_the_link() {
	local(|| {
		async {
			let mut shown = show(
				Some(true),
				|_| {},
				true,
				|_| route_device::page(&route_device::Device { user_code: None }),
			)
			.await;
			assert!(shown.has("Connect a device"));
			shown
				.tester
				.enter_text(&Finder::label("Device code"), " wdjb-mjht?<x> ")
				.unwrap();
			shown.tap(&Finder::text("Continue")).await;
			// Looked up as the server reads it; with no server, the error
			// shows.
			assert!(shown.has(ALERT), "{}", shown.text());
			assert!(shown.has("Try again"), "{}", shown.text());

			let mut shown = show(
				Some(true),
				|cache| {
					cache.set_query_data(
						&client::device_request(),
						"WDJB-MJHT".into(),
						leptoskit::idp::DeviceRequest {
							app: partner(),
							scopes: scopes(),
							user_code: "WDJB-MJHT".into(),
						},
					);
				},
				true,
				|_| {
					route_device::page(&route_device::Device {
						user_code: Some("wdjb-mjht".into()),
					})
				},
			)
			.await;
			assert!(
				shown.has(
					"Check that your device shows WDJB-MJHT, then let Partner use your account."
				),
				"{}",
				shown.text()
			);
			assert!(shown.has("• notes:read"), "{}", shown.text());
			shown.tap(&Finder::text("Allow")).await;
			assert!(shown.has(ALERT), "{}", shown.text());
			shown.tap(&Finder::text("Cancel")).await;
			assert!(shown.has(ALERT), "{}", shown.text());

			// An empty code asks again, and so does a signed-out visit.
			shown.screens.device_code.open(Some("  "));
			shown.settle().await;
			let shown = show(
				Some(false),
				|_| {},
				true,
				|_| {
					route_device::page(&route_device::Device {
						user_code: Some("WDJB-MJHT".into()),
					})
				},
			)
			.await;
			assert!(shown.has("Email address"), "{}", shown.text());
		}
	})
	.await;
}

#[tokio::test]
async fn connected_apps_are_listed_and_can_be_disconnected() {
	local(|| {
		async {
			let now = chrono::Utc::now();
			let mut shown = show(
				Some(true),
				|cache| {
					cache.set_query_data(
						&client::connected_apps(),
						(),
						vec![
							leptoskit::idp::ConnectedApp {
								app: partner(),
								scopes: scopes(),
								connected_at: now,
								last_used_at: None,
							},
							leptoskit::idp::ConnectedApp {
								app: leptoskit::idp::AppInfo {
									client_id: "cli".into(),
									name: "Notes CLI".into(),
								},
								scopes: scopes(),
								connected_at: now,
								last_used_at: Some(now),
							},
						],
					);
				},
				true,
				|screens| screens.gate(|| view! { <Text>"Home"</Text> }),
			)
			.await;
			assert!(shown.has("Home"), "{}", shown.text());
			shown.tap(&Finder::text("Connected apps")).await;
			assert!(
				shown.has("Sign you in with your account. notes:read"),
				"{}",
				shown.text()
			);
			assert!(shown.has("Last used"), "{}", shown.text());
			assert!(shown.has("Connected "), "{}", shown.text());
			shown.tap(&Finder::label("Disconnect Partner")).await;
			assert!(shown.has(ALERT), "{}", shown.text());

			let shown = show(
				Some(true),
				|cache| cache.set_query_data(&client::connected_apps(), (), Vec::new()),
				true,
				|_| route_connected_apps::page(),
			)
			.await;
			assert!(
				shown.has("No apps can use your account"),
				"{}",
				shown.text()
			);

			// Unseeded, the read fails without a server and offers to try
			// again.
			let mut shown = show(Some(true), |_| {}, true, |_| route_connected_apps::page()).await;
			assert!(shown.has(ALERT), "{}", shown.text());
			shown.tap(&Finder::text("Try again")).await;
			assert!(shown.has(ALERT), "{}", shown.text());

			let shown = show(Some(false), |_| {}, true, |_| route_connected_apps::page()).await;
			assert!(shown.has("Email address"), "{}", shown.text());
		}
	})
	.await;
}

#[tokio::test]
async fn the_portal_lists_apps_and_registers_more() {
	local(|| {
		async {
			let app = |name: &str, auth_method| {
				leptoskit::idp::ClientInfo {
					client_id: format!("{name}-id"),
					name: name.into(),
					auth_method,
					grant_types: leptoskit::idp::default_grant_types(),
					jwks: None,
					id_token_alg: leptoskit::idp::SigningAlgorithm::Es256,
					redirect_uris: vec!["https://partner.example/cb".into()],
					post_logout_redirect_uris: Vec::new(),
					scopes: vec!["openid".into()],
					first_party: false,
					introspection: false,
					created_at: chrono::Utc::now(),
				}
			};
			let mut shown = show(
				Some(true),
				|cache| {
					cache.set_query_data(
						&client::my_apps(),
						(),
						vec![
							app("Secret", leptoskit::idp::ClientAuth::SecretBasic),
							app("Public", leptoskit::idp::ClientAuth::None),
						],
					);
				},
				true,
				|_| route_developer_apps::page(),
			)
			.await;
			assert!(shown.has("Client ID: Secret-id"), "{}", shown.text());
			assert!(
				shown
					.tester
					.find(&Finder::label("New secret for Public"))
					.is_err()
			);
			shown.tap(&Finder::label("New secret for Secret")).await;
			assert!(shown.has(ALERT), "{}", shown.text());
			shown.tap(&Finder::label("Delete Public")).await;
			assert!(shown.has(ALERT), "{}", shown.text());

			shown.tap(&Finder::text("Register with a secret")).await;
			assert!(
				shown.has("Give the app a name and a redirect URI."),
				"{}",
				shown.text()
			);
			shown
				.tester
				.enter_text(&Finder::label("App name"), "Mine")
				.unwrap();
			shown
				.tester
				.enter_text(&Finder::label("Redirect URI"), "https://mine.example/cb")
				.unwrap();
			shown
				.tester
				.enter_text(&Finder::label("Scopes"), "openid profile")
				.unwrap();
			shown.tap(&Finder::text("Register a public app")).await;
			assert!(shown.has(ALERT), "{}", shown.text());
			assert!(!shown.has("Give the app a name"), "{}", shown.text());

			// The apps are read only once the screen is shown, and fail here.
			let mut shown = show(
				Some(true),
				|_| {},
				true,
				|screens| screens.developer_apps.screen(),
			)
			.await;
			assert!(shown.has(ALERT), "{}", shown.text());
			shown.tap(&Finder::text("Try again")).await;
			assert!(shown.has(ALERT), "{}", shown.text());

			let shown = show(Some(false), |_| {}, true, |_| route_developer_apps::page()).await;
			assert!(shown.has("Email address"), "{}", shown.text());
		}
	})
	.await;
}

#[tokio::test]
async fn signing_in_goes_on_to_return_to() {
	local(|| {
		async {
			let mut shown = show(
				Some(false),
				|_| {},
				true,
				|_| {
					route_sign_in_idp::page(&route_sign_in_idp::SignIn {
						return_to: Some("/oauth/authorize?client_id=partner".into()),
					})
				},
			)
			.await;
			assert!(shown.has("Email address"), "{}", shown.text());
			assert!(shown.screens.is_returning());

			// The sign-in completes: the session query now holds a session.
			shown
				.cache
				.set_query_data(&leptoskit::auth::client::session(), (), Some(session()));
			shown.settle().await;
			assert_eq!(
				*shown.visited.lock().unwrap(),
				["/oauth/authorize?client_id=partner"]
			);
		}
	})
	.await;
}

#[tokio::test]
async fn someone_already_signed_in_chooses_to_go_on() {
	local(|| {
		async {
			let mut shown = show(
				Some(true),
				|_| {},
				true,
				|_| {
					route_sign_in_idp::page(&route_sign_in_idp::SignIn {
						return_to: Some("/oauth/authorize?prompt=login".into()),
					})
				},
			)
			.await;
			// Already signed in: nothing moved on by itself, so an app asking
			// them to sign in again can't loop.
			assert!(shown.visited.lock().unwrap().is_empty());
			assert!(shown.has("You're signed in."), "{}", shown.text());
			shown.tap(&Finder::text("Sign in as someone else")).await;
			shown.tap(&Finder::text("Continue")).await;
			assert_eq!(
				*shown.visited.lock().unwrap(),
				["/oauth/authorize?prompt=login"]
			);

			// Another site's address is never kept, and without it, or before
			// the app says how to leave, nothing happens.
			let shown = show(
				Some(true),
				|_| {},
				false,
				|_| {
					route_sign_in_idp::page(&route_sign_in_idp::SignIn {
						return_to: Some("//evil.example".into()),
					})
				},
			)
			.await;
			assert!(!shown.screens.is_returning());
			assert!(shown.has("Sign out"), "{}", shown.text());
			shown.screens.return_to(Some("/home"));
			shown.screens.continue_on();
			assert!(!shown.screens.is_returning());
			assert!(shown.visited.lock().unwrap().is_empty());
		}
	})
	.await;
}

#[tokio::test]
async fn the_other_routes_show_their_screens() {
	local(|| {
		async {
			for (signed_in, expected) in
				[(true, "Where you're signed in"), (false, "Email address")]
			{
				let shown = show(Some(signed_in), |_| {}, true, |_| route_devices::page()).await;
				assert!(shown.has(expected), "{}", shown.text());
			}

			for (signed_in, expected) in [(true, "Security"), (false, "Email address")] {
				let shown = show(Some(signed_in), |_| {}, true, |_| route_security::page()).await;
				assert!(shown.has(expected), "{}", shown.text());
			}

			let shown = show(Some(false), |_| {}, true, |_| route_sign_in::page()).await;
			assert!(shown.has("Email address"), "{}", shown.text());
		}
	})
	.await;
}
