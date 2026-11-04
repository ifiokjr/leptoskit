#![cfg(not(target_arch = "wasm32"))]

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
use leptoskit::auth::MfaStatus;
use leptoskit::auth::PasswordSettings;
use leptoskit::auth::PublicSession;
use leptoskit::idp::client;
use leptoskit::native::NativeKind;
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
	owner: Owner,
}

impl Shown {
	fn text(&self) -> String {
		self.tester.semantics()
	}

	fn has(&self, text: &str) -> bool {
		self.text().contains(text)
	}

	async fn tap(&mut self, finder: &Finder) {
		self.owner.with(|| self.tester.tap(finder)).unwrap();
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
		owner,
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
					dpop_bound: false,
					self_registered: false,
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

fn seed_security(cache: &QueryClient) {
	use leptoskit::auth::client as auth_client;
	cache.set_query_data(&auth_client::mfa(), (), MfaStatus::default());
	cache.set_query_data(&auth_client::passkeys(), (), Vec::new());
	cache.set_query_data(&auth_client::identities(), (), Vec::new());
	cache.set_query_data(&auth_client::providers(), (), Vec::new());
	cache.set_query_data(&auth_client::merge_offer(), (), None);
	cache.set_query_data(&auth_client::wallets(), (), Vec::new());
	cache.set_query_data(
		&auth_client::wallet_settings(),
		(),
		leptoskit::auth::WalletSettings {
			enabled: false,
			sign_in: false,
			chain_id: "mainnet".into(),
		},
	);
	cache.set_query_data(&auth_client::wallet_apps(), (), Vec::new());
}

#[tokio::test]
async fn security_lists_sign_in_methods_and_preserves_them_when_changes_fail() {
	local(|| async {
		use leptoskit::auth::client as auth_client;
		let now = chrono::Utc::now();
		let mut shown = show(Some(true), |cache| {
			seed_security(cache);
			cache.set_query_data(&auth_client::mfa(), (), MfaStatus {
				totp: true, recovery_codes: 3, passkeys: 2, ..Default::default()
			});
			cache.set_query_data(&auth_client::passkeys(), (), vec![
				leptoskit::auth::PasskeyInfo {
					id: leptoskit::db::Id::parse("00000000-0000-4000-8000-000000000011").unwrap(),
					name: "Laptop".into(), backed_up: true, created_at: now, last_used_at: Some(now),
				},
				leptoskit::auth::PasskeyInfo {
					id: leptoskit::db::Id::parse("00000000-0000-4000-8000-000000000012").unwrap(),
					name: "Security key".into(), backed_up: false, created_at: now, last_used_at: None,
				},
			]);
			cache.set_query_data(&auth_client::providers(), (), serde_json::from_value(serde_json::json!([
				{"id":"github", "name":"GitHub"}, {"id":"google", "name":"Google"}
			])).unwrap());
			cache.set_query_data(&auth_client::identities(), (), serde_json::from_value(serde_json::json!([
				{"id":"00000000-0000-4000-8000-000000000021", "provider":"github", "provider_name":"GitHub", "name":"Ada", "email":"ada@example.com", "created_at":now}
			])).unwrap());
		}, true, |screens| screens.security.screen()).await;
		for expected in ["Synced to your other devices", "On one device", "Not used yet", "Last used", "You have 3 recovery codes left.", "Ada"] {
			assert!(shown.has(expected), "missing {expected}: {}", shown.text());
		}
		for button in ["Remove Laptop", "Remove Security key", "Disconnect GitHub"] {
			shown.tap(&Finder::label(button)).await;
			assert!(shown.has(ALERT), "{}", shown.text());
		}
		for button in ["Connect Google", "Add a passkey", "Make new recovery codes", "Turn off the authenticator app"] {
			shown.tap(&Finder::text(button)).await;
			assert!(shown.has(ALERT), "{}", shown.text());
		}
		assert!(shown.has("Laptop") && shown.has("Security key") && shown.has("Ada"));
	}).await;
}

#[tokio::test]
async fn security_explains_required_factors_and_recovery_code_counts() {
	local(|| {
		async {
			use leptoskit::auth::client as auth_client;
			let mut shown = show(
				Some(true),
				|cache| {
					seed_security(cache);
					cache.set_query_data(
						&auth_client::mfa(),
						(),
						MfaStatus {
							required: true,
							..Default::default()
						},
					);
				},
				true,
				|screens| screens.security.screen(),
			)
			.await;
			assert!(shown.has("Your account must use two-step verification."));
			shown
				.tap(&Finder::text("Set up an authenticator app"))
				.await;
			assert!(shown.has(ALERT));
			assert!(shown.has("No authenticator app is set up."));
			for (recovery_codes, expected) in [
				(0, "You have no recovery codes left."),
				(1, "You have 1 recovery code left."),
			] {
				shown.cache.set_query_data(
					&auth_client::mfa(),
					(),
					MfaStatus {
						totp: true,
						recovery_codes,
						..Default::default()
					},
				);
				shown.settle().await;
				assert!(shown.has(expected), "{}", shown.text());
			}
		}
	})
	.await;
}

#[tokio::test]
async fn security_merge_preview_explains_data_choices_before_confirmation() {
	local(|| {
		async {
			use leptoskit::auth::Factor;
			use leptoskit::auth::MergeData;
			use leptoskit::auth::MergeFate;
			use leptoskit::auth::MergeMode;
			use leptoskit::auth::MergeOffer;
			use leptoskit::auth::client as auth_client;
			for provider in ["github", "solana"] {
				let mut shown = show(
					Some(true),
					|cache| {
						seed_security(cache);
						cache.set_query_data(
							&auth_client::merge_offer(),
							(),
							Some(MergeOffer {
								provider: provider.into(),
								created_at: chrono::Utc::now(),
								emails: vec!["other@example.com".into()],
								identities: vec!["GitHub".into()],
								passkeys: 1,
								wallets: vec![],
								data: vec![
									MergeData {
										label: "posts".into(),
										count: 12,
										fate: MergeFate::Moves,
									},
									MergeData {
										label: "drafts".into(),
										count: 1,
										fate: MergeFate::Dropped,
									},
								],
								second_factor: vec![Factor::Totp, Factor::Passkey],
								modes: vec![MergeMode::BringData, MergeMode::MoveLogin],
								expires_in: 300,
							}),
						);
					},
					true,
					|screens| screens.security.screen(),
				)
				.await;
				assert!(shown.has("already has its own account"));
				assert!(
					shown.has("Moves 12 posts. Drops 1 drafts, since yours stay."),
					"{}",
					shown.text()
				);
				assert!(shown.has("other@example.com") && shown.has("1 passkey"));
				shown
					.tap(&Finder::text(
						"○ Move only the sign-in methods, and delete its data",
					))
					.await;
				assert!(shown.has("Its data is deleted with it."));
				shown.tap(&Finder::text("Merge accounts")).await;
				assert!(shown.has(ALERT), "{}", shown.text());
				shown.tap(&Finder::text("Use its passkey")).await;
				assert!(shown.has(ALERT));
				shown.tap(&Finder::text("Cancel")).await;
				assert!(shown.has("already has its own account"));
			}
		}
	})
	.await;
}

#[tokio::test]
async fn linked_wallet_controls_explain_visibility_and_report_refused_changes() {
	local(|| async {
		use leptoskit::auth::client as auth_client;
		let now = chrono::Utc::now();
		let mut shown = show(Some(true), |cache| {
			seed_security(cache);
			cache.set_query_data(&auth_client::wallet_settings(), (), leptoskit::auth::WalletSettings {
				enabled: true, sign_in: true, chain_id: "mainnet".into(),
			});
			cache.set_query_data(&auth_client::wallets(), (), serde_json::from_value(serde_json::json!([
				{"id":"00000000-0000-4000-8000-000000000031", "chain":"solana", "address":"11111111111111111111111111111111", "label":"Treasury", "can_sign_in":true, "is_public":true, "verified_at":now, "last_proved_at":now},
				{"id":"00000000-0000-4000-8000-000000000032", "chain":"solana", "address":"11111111111111111111111111111112", "label":null, "can_sign_in":false, "is_public":false, "verified_at":now, "last_proved_at":now}
			])).unwrap());
			cache.set_query_data(&auth_client::wallet_apps(), (), vec![auth_client::WalletApp {
				name: "Phantom".into(), icon: String::new(), sign_in: true,
			}]);
		}, true, |screens| screens.security.screen()).await;
		for expected in ["Treasury", "Public", "Private", "Can sign in", "1111…1112"] {
			assert!(shown.has(expected), "{}", shown.text());
		}
		for label in ["Hide Treasury on your profile", "Show 1111…1112 on your profile", "Stop signing in with Treasury", "Allow signing in with 1111…1112", "Unlink Treasury", "Unlink 1111…1112"] {
			shown.tap(&Finder::label(label)).await;
		}
		shown.tap(&Finder::text("Link a wallet with Phantom")).await;
		assert!(shown.has(ALERT), "{}", shown.text());
		assert!(shown.has("Treasury"));
		shown.cache.set_query_data(&auth_client::wallet_apps(), (), Vec::new());
		shown.settle().await;
		assert!(shown.has("Install a Solana wallet"), "{}", shown.text());
	}).await;
}

#[tokio::test]
async fn the_device_list_marks_this_device_and_reports_failed_sign_outs() {
	local(|| {
		async {
			use leptoskit::auth::client as auth_client;
			let mut current = session();
			current.device = "This laptop".into();
			let mut other = current.clone();
			other.id = leptoskit::db::Id::parse("00000000-0000-4000-8000-000000000041").unwrap();
			other.device = "Lost phone".into();
			let mut unknown = other.clone();
			unknown.id = leptoskit::db::Id::parse("00000000-0000-4000-8000-000000000042").unwrap();
			unknown.device = " ".into();
			let mut shown = show(
				Some(true),
				|cache| {
					cache.set_query_data(
						&auth_client::devices(),
						(),
						vec![
							leptoskit::auth::Device {
								session: current,
								current: true,
							},
							leptoskit::auth::Device {
								session: other,
								current: false,
							},
							leptoskit::auth::Device {
								session: unknown,
								current: false,
							},
						],
					);
				},
				true,
				|screens| screens.devices.screen(),
			)
			.await;
			for expected in [
				"This device",
				"This laptop",
				"Lost phone",
				"An unknown device",
			] {
				assert!(shown.has(expected), "{}", shown.text());
			}
			shown.tap(&Finder::label("Sign out Lost phone")).await;
			assert!(shown.has(ALERT), "{}", shown.text());
			for button in ["Sign out of every other device", "Sign out everywhere"] {
				shown.tap(&Finder::text(button)).await;
				assert!(shown.has(ALERT));
			}
			assert!(shown.has("Lost phone"));
		}
	})
	.await;
}

/// A server that takes passwords of 8 to 64 characters, or doesn't.
fn passwords(cache: &QueryClient, enabled: bool) {
	cache.set_query_data(
		&leptoskit::auth::client::password_settings(),
		(),
		PasswordSettings {
			enabled,
			min_length: 8,
			max_length: 64,
		},
	);
}

/// The security screen's status for an account with or without a password.
fn status(password: bool) -> MfaStatus {
	serde_json::from_value(serde_json::json!({
		"totp": false,
		"recovery_codes": 0,
		"passkeys": 0,
		"required": false,
		"password": password,
	}))
	.unwrap()
}

/// A field's kind and `autocomplete` hint, found by its accessible name.
fn field(shown: &Shown, label: &str) -> (NativeKind, Option<String>) {
	let widget = shown.tester.find(&Finder::label(label)).unwrap();
	(widget.kind, widget.props.autocomplete)
}

#[tokio::test]
async fn a_server_with_passwords_offers_password_sign_in_in_masked_fields() {
	local(|| {
		async {
			let mut shown = show(
				Some(false),
				|cache| passwords(cache, true),
				true,
				|screens| screens.sign_in.screen(),
			)
			.await;
			shown.tap(&Finder::text("Sign in with a password")).await;
			assert_eq!(
				field(&shown, "Email address"),
				(NativeKind::TextInput, Some("username".into()))
			);
			assert_eq!(
				field(&shown, "Password"),
				(NativeKind::PasswordInput, Some("current-password".into()))
			);

			// What's missing is said before anything is sent.
			shown
				.tap(&Finder::role(NativeKind::Button, "Sign in"))
				.await;
			assert!(shown.has("Enter an email address"), "{}", shown.text());
			shown
				.tester
				.enter_text(&Finder::label("Email address"), "ada@example.com")
				.unwrap();
			shown
				.tap(&Finder::role(NativeKind::Button, "Sign in"))
				.await;
			assert!(shown.has("Enter your password."), "{}", shown.text());

			// Without a server, the sign-in is refused, and the screen says so.
			shown
				.tester
				.enter_text(&Finder::label("Password"), "correct horse battery")
				.unwrap();
			shown
				.tap(&Finder::role(NativeKind::Button, "Sign in"))
				.await;
			assert!(shown.has(ALERT), "{}", shown.text());
			assert!(
				!shown.tester.tree().contains("correct horse battery"),
				"{}",
				shown.tester.tree()
			);

			// A forgotten password needs a code, which can't be sent here.
			shown.tap(&Finder::text("Forgot your password?")).await;
			assert!(shown.has(ALERT), "{}", shown.text());
			assert!(shown.has("Password"), "{}", shown.text());

			shown.tap(&Finder::text("Email me a code instead")).await;
			assert!(shown.has("Email me a code"), "{}", shown.text());
			assert!(shown.tester.find(&Finder::label("Password")).is_err());

			// A server without passwords offers none.
			let shown = show(
				Some(false),
				|cache| passwords(cache, false),
				true,
				|screens| screens.sign_in.screen(),
			)
			.await;
			assert!(shown.has("Email me a code"), "{}", shown.text());
			assert!(!shown.has("Sign in with a password"), "{}", shown.text());
		}
	})
	.await;
}

#[tokio::test]
async fn the_security_screen_sets_changes_and_removes_a_password() {
	local(|| {
		async {
			let mut shown = show(
				Some(true),
				|cache| {
					passwords(cache, true);
					cache.set_query_data(&leptoskit::auth::client::mfa(), (), status(false));
				},
				true,
				|screens| screens.security.screen(),
			)
			.await;
			assert!(shown.has("Use at least 8 characters."), "{}", shown.text());
			assert_eq!(
				field(&shown, "Password"),
				(NativeKind::PasswordInput, Some("new-password".into()))
			);
			assert!(shown.tester.find(&Finder::text("Remove password")).is_err());

			// Too short is caught here; the server checks the rest.
			shown
				.tester
				.enter_text(&Finder::label("Password"), "short")
				.unwrap();
			shown.tap(&Finder::text("Set a password")).await;
			assert!(
				shown.has("alert \"Use at least 8 characters."),
				"{}",
				shown.text()
			);
			shown
				.tester
				.enter_text(&Finder::label("Password"), "a long enough phrase")
				.unwrap();
			shown.tap(&Finder::text("Set a password")).await;
			assert!(shown.has(ALERT), "{}", shown.text());

			// An account with a password changes or removes it.
			shown
				.cache
				.set_query_data(&leptoskit::auth::client::mfa(), (), status(true));
			shown.settle().await;
			assert_eq!(
				field(&shown, "New password"),
				(NativeKind::PasswordInput, Some("new-password".into()))
			);
			assert!(shown.has("Change password"), "{}", shown.text());
			shown.tap(&Finder::text("Remove password")).await;
			assert!(shown.has(ALERT), "{}", shown.text());

			// Without passwords on the server, there's no password section.
			let shown = show(
				Some(true),
				|cache| {
					passwords(cache, false);
					cache.set_query_data(&leptoskit::auth::client::mfa(), (), status(false));
				},
				true,
				|screens| screens.security.screen(),
			)
			.await;
			assert!(shown.has("Passkeys"), "{}", shown.text());
			assert!(!shown.has("Set a password"), "{}", shown.text());
		}
	})
	.await;
}
