//! Generated security screens use the real browser auth transport against a
//! deterministic fetch fixture. Requests and visible outcomes are both checked.
#![cfg(all(target_arch = "wasm32", feature = "web"))]

use auth_pages_example::auth;
use leptoskit::auth::client;
use leptoskit::query::NetworkMode;
use leptoskit::query::QueryClient;
use leptoskit::query::QueryDefaults;
use leptoskit::query::RefetchOn;
use leptoskit::query::RetryPolicy;
use leptoskit_testing::Finder;
use leptoskit_testing::Queries;
use leptoskit_testing::WidgetTester;
use reactive_graph::owner::Owner;
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen(module = "/tests/support/auth_server.js")]
extern "C" {
	fn install(signed_in: bool);
	fn factors(json: &str);
	fn enable_wallets();
	fn wallet_mode(mode: &str);
	fn enable_passkeys();
	fn passkey_mode(mode: &str);
	fn restore();
	fn calls() -> String;
}

#[derive(Clone, Copy)]
enum Kind {
	Security,
	SignIn,
	Huddle,
}

struct Screen {
	tester: WidgetTester,
	cache: QueryClient,
	owner: Owner,
}

impl Screen {
	async fn new(kind: Kind) -> Self {
		let _ = any_spawner::Executor::init_wasm_bindgen();
		client::configure(client::ClientConfig::new());
		let owner = Owner::new();
		let (render, cache) = owner.with(|| {
			let cache = QueryClient::new().with_defaults(
				QueryDefaults::new()
					.with_retry(RetryPolicy::Never)
					.with_network_mode(NetworkMode::Always)
					.with_stale_time(std::time::Duration::from_secs(3600))
					.with_refetch_on_window_focus(RefetchOn::Never),
			);
			cache.provide();
			let render: Box<dyn Fn() -> leptoskit::prelude::Element> = match kind {
				Kind::Security => {
					let screens = auth::provide();
					Box::new(move || screens.security.screen())
				}
				Kind::SignIn => {
					let screen = auth::SignIn::new();
					Box::new(move || screen.screen())
				}
				Kind::Huddle => {
					let screen = huddle_app::auth::SignIn::new();
					Box::new(move || screen.screen())
				}
			};
			(render, cache)
		});
		let drawing = owner.clone();
		let tester = WidgetTester::new(move || drawing.with(&render), 800.0, 1000.0).unwrap();
		let mut shown = Self {
			tester,
			cache,
			owner,
		};
		shown.settle().await;
		shown
	}

	async fn settle(&mut self) {
		for round in 0..100 {
			gloo_timers::future::TimeoutFuture::new(5).await;
			self.tester.pump();
			if round >= 5 && self.cache.fetching_count() == 0 && self.cache.mutating_count() == 0 {
				return;
			}
		}
		panic!(
			"screen did not settle: fetching={}, mutating={}, requests={}\n{}",
			self.cache.fetching_count(),
			self.cache.mutating_count(),
			calls(),
			self.tester.semantics()
		);
	}

	fn has(&self, text: &str) {
		let tree = self.tester.semantics();
		assert!(tree.contains(text), "missing {text}: {tree}");
	}

	async fn tap(&mut self, text: &str) {
		self.tester
			.tap(&Finder::role(leptoskit::native::NativeKind::Button, text))
			.unwrap();
		self.settle().await;
	}

	async fn enter(&mut self, label: &str, text: &str) {
		self.tester.enter_text(&Finder::label(label), text).unwrap();
		self.settle().await;
	}

	fn finish(self) {
		drop(self.tester);
		self.owner.cleanup();
	}
}

fn assert_posted(path: &str, field: &str, value: &str) {
	let requests: serde_json::Value = serde_json::from_str(&calls()).unwrap();
	assert!(
		requests.as_array().unwrap().iter().any(|call| {
			call["method"] == "POST" && call["path"] == path && call["body"][field] == value
		}),
		"missing POST {path} with {field}={value}: {requests}"
	);
}

#[wasm_bindgen_test]
async fn authenticator_setup_recovery_and_step_up_follow_the_server() {
	install(true);
	let mut shown = Screen::new(Kind::Security).await;
	shown.has("No authenticator app is set up.");
	shown.tap("Set up an authenticator app").await;
	shown.has("Setup key");
	assert_eq!(
		shown
			.tester
			.get_by_label("Setup key")
			.unwrap()
			.props
			.text
			.as_deref(),
		Some("JBSW Y3DP EHPK 3PXP")
	);
	shown.tap("Turn on").await;
	shown.has("Enter the code your app shows.");
	shown.enter("Code from the app", " 123456 ").await;
	shown.tap("Turn on").await;
	shown.has("FIRST-RECOVERY");
	shown.tap("I've saved them").await;
	shown.has("An authenticator app is on.");
	shown.has("You have 2 recovery codes left.");
	assert!(!shown.tester.semantics().contains("FIRST-RECOVERY"));
	shown.tap("Make new recovery codes").await;
	shown.has("NEW-RECOVERY");
	shown.tap("I've saved them").await;
	shown.tap("Turn off the authenticator app").await;
	shown.has("Confirm it's you before this change.");
	shown.tap("Confirm with the code").await;
	shown.has("Enter a code to confirm it's you.");
	shown.enter("Authenticator or recovery code", "wrong").await;
	shown.tap("Confirm with the code").await;
	shown.has("That code was refused.");
	shown
		.enter("Authenticator or recovery code", "654321")
		.await;
	shown.tap("Confirm with the code").await;
	shown.has("No authenticator app is set up.");
	let requests: serde_json::Value = serde_json::from_str(&calls()).unwrap();
	let posts: Vec<_> = requests
		.as_array()
		.unwrap()
		.iter()
		.filter(|call| call["method"] == "POST")
		.collect();
	assert!(
		posts
			.iter()
			.any(|call| call["path"] == "/auth/totp/confirm" && call["body"]["code"] == "123456")
	);
	assert_eq!(
		posts
			.iter()
			.filter(|call| call["path"] == "/auth/totp/disable")
			.count(),
		2,
		"the refused change is retried after step-up"
	);
	shown.finish();
	restore();
}

#[wasm_bindgen_test]
async fn generated_and_huddle_sign_in_screens_complete_both_factors() {
	for kind in [Kind::SignIn, Kind::Huddle] {
		install(false);
		let mut shown = Screen::new(kind).await;
		shown.tap("Email me a code").await;
		shown.has("Enter an email address, such as ada@example.com.");
		shown.enter("Email address", "  ada@example.com  ").await;
		shown.tap("Email me a code").await;
		shown.has("Check ada@example.com for a code.");
		shown.tap("Sign in").await;
		shown.has("Enter the code from the email.");
		shown.tap("Send a new code").await;
		shown.tap("Use a different address").await;
		shown.has("Email address");
		shown.enter("Email address", "ada@example.com").await;
		shown.tap("Email me a code").await;
		shown.enter("Code", "bad").await;
		shown.tap("Sign in").await;
		shown.has("That code was refused.");
		shown.enter("Code", " ABCD-EFGH ").await;
		shown.tap("Sign in").await;
		shown.has("Lost your phone?");
		shown.tap("Continue").await;
		shown.has("Enter the code from your authenticator app.");
		shown
			.enter("Authenticator or recovery code", "expired")
			.await;
		shown.tap("Continue").await;
		shown.has("That took too long. Enter your email address to start again.");
		for (allowed, expected) in [
			(
				r#"["passkey","recovery_code"]"#,
				"Use your passkey, or enter one of your recovery codes.",
			),
			(
				r#"["totp"]"#,
				"Enter the 6-digit code from your authenticator app.",
			),
		] {
			complete_email_sign_in(&mut shown, allowed, expected).await;
		}
		shown.enter("Email address", "ada@example.com").await;
		shown.tap("Email me a code").await;
		shown.enter("Code", "DIRECT").await;
		shown.tap("Sign in").await;
		shown.has("You're signed in.");
		assert_posted("/auth/second-factor", "code", "654321");
		assert_posted("/auth/email/send", "email", "ada@example.com");
		shown.finish();
		restore();
	}
}

async fn complete_email_sign_in(shown: &mut Screen, allowed: &str, expected: &str) {
	factors(allowed);
	shown.enter("Email address", "ada@example.com").await;
	shown.tap("Email me a code").await;
	shown.enter("Code", "ABCD-EFGH").await;
	shown.tap("Sign in").await;
	shown.has(expected);
	shown
		.enter("Authenticator or recovery code", " 654321 ")
		.await;
	shown.tap("Continue").await;
	shown.has("You're signed in.");
	shown.tap("Sign out").await;
	shown.has("Email address");
}

#[wasm_bindgen_test]
async fn browser_passkeys_register_sign_in_and_explain_platform_refusals() {
	install(true);
	enable_passkeys();
	let mut shown = Screen::new(Kind::Security).await;
	shown.tap("Add a passkey").await;
	shown.has("Browser passkey");
	shown.has("Synced to your other devices");
	shown
		.tester
		.tap(&Finder::label("Remove Browser passkey"))
		.unwrap();
	shown.settle().await;
	assert!(!shown.tester.semantics().contains("Browser passkey"));
	let add = shown
		.owner
		.with(|| leptoskit::query::use_mutation(client::add_passkey()));
	for (mode, expected) in [
		("NotAllowedError", "cancelled"),
		("AbortError", "cancelled"),
		("NotSupportedError", "unsupported"),
		("SecurityError", "unsupported"),
		("UnknownError", "passkey_failed"),
	] {
		passkey_mode(mode);
		assert_eq!(add.mutate_async(None).await.unwrap_err().error, expected);
	}
	shown.finish();
	restore();
	for kind in [Kind::SignIn, Kind::Huddle] {
		install(false);
		enable_passkeys();
		let mut shown = Screen::new(kind).await;
		passkey_mode("NotAllowedError");
		shown.tap("Sign in with a passkey").await;
		shown.has("No passkey was used.");
		passkey_mode("ok");
		shown.tap("Sign in with a passkey").await;
		shown.has("You're signed in.");
		shown.tap("Sign out").await;
		shown.enter("Email address", "ada@example.com").await;
		shown.tap("Email me a code").await;
		shown.enter("Code", "ABCD-EFGH").await;
		shown.tap("Sign in").await;
		shown.tap("Use a passkey").await;
		shown.has("You're signed in.");
		let requests: serde_json::Value = serde_json::from_str(&calls()).unwrap();
		for path in ["/auth/passkey/sign-in", "/auth/passkey/second-factor"] {
			assert!(requests.as_array().unwrap().iter().any(|call| {
				call["path"] == path && call["body"]["credential"]["rawId"] == "AQID"
			}));
		}
		shown.finish();
		restore();
	}
}

#[wasm_bindgen_test]
async fn wallet_standard_links_both_kinds_and_rejects_invalid_responses() {
	install(true);
	enable_wallets();
	let mut shown = Screen::new(Kind::Security).await;
	let apps = shown
		.cache
		.fetch_query(&client::wallet_apps(), ())
		.await
		.unwrap();
	assert_eq!(
		apps.iter().map(|app| app.name.as_str()).collect::<Vec<_>>(),
		["Fixture SIWS", "Fixture Text"]
	);
	shown.tap("Link a wallet with Fixture SIWS").await;
	shown.has("It's linked.");
	shown.has("Fixture account");
	shown.tap("Link a wallet with Fixture Text").await;
	shown.has("That wallet was already linked.");
	let prove = shown
		.owner
		.with(|| leptoskit::query::use_mutation(client::prove_wallet()));
	let input = client::ProveWalletInput {
		wallet: "Fixture Text".into(),
		address: leptoskit::db::Pubkey::parse("11111111111111111111111111111111").unwrap(),
	};
	assert_eq!(
		prove
			.mutate_async(input.clone())
			.await
			.unwrap()
			.label
			.as_deref(),
		Some("Fixture account")
	);
	for (mode, expected) in [
		("cancel", "cancelled"),
		("prefix", "wallet_failed"),
		("short", "wallet_failed"),
		("no_account", "wallet_failed"),
	] {
		wallet_mode(mode);
		assert_eq!(
			prove.mutate_async(input.clone()).await.unwrap_err().error,
			expected
		);
	}
	wallet_mode("ok");
	let link = shown
		.owner
		.with(|| leptoskit::query::use_mutation(client::link_wallet()));
	let missing = link
		.mutate_async(client::LinkWalletInput {
			wallet: "Missing wallet".into(),
			label: None,
		})
		.await
		.unwrap_err();
	assert_eq!(missing.error, "no_wallet");
	shown
		.tester
		.tap(&Finder::label("Show Fixture account on your profile"))
		.unwrap();
	shown.settle().await;
	shown.has("Public");
	shown
		.tester
		.tap(&Finder::label("Allow signing in with Fixture account"))
		.unwrap();
	shown.settle().await;
	shown.has("Can sign in");
	shown
		.tester
		.tap(&Finder::label("Unlink Fixture account"))
		.unwrap();
	shown.settle().await;
	assert!(!shown.tester.semantics().contains("Fixture account"));
	let requests: serde_json::Value = serde_json::from_str(&calls()).unwrap();
	assert!(requests.as_array().unwrap().iter().any(|call| {
		call["path"] == "/auth/wallets/link"
			&& call["body"]["proof"]["message"] == "fixture SIWS message"
	}));
	assert!(requests.as_array().unwrap().iter().any(|call| {
		call["path"] == "/auth/wallets/link"
			&& call["body"]["proof"]["message"]
				.as_str()
				.is_some_and(|text| text.contains("wants you to sign in with your Solana account"))
	}));
	shown.finish();
	restore();
}
