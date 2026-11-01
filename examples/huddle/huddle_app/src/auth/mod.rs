//! Signing in, in front of everything else.
//!
//! `sign_in.rs` is the screen `leptoskit generate auth` writes: an email
//! address, then the code emailed to it. In development the code is at
//! `/_leptoskit/mail` on the server. A debug build in the browser adds
//! buttons that sign in as one of three teammates at once, through the
//! server's dev sign-in route, so two browser windows can be two people in
//! seconds.

mod sign_in;

use leptoskit::prelude::*;
use leptoskit::view;
pub use sign_in::SignIn;

/// The sign-in screen, centred, with the development shortcuts under it.
pub fn screen(sign_in: SignIn) -> Element {
	view! {
		<ScrollView class="min-h-screen bg-slate-950 text-white">
			<View class="items-center gap-6 p-6">
				<View class="items-center gap-1 pt-8">
					<Text class="text-3xl font-bold">"Huddle"</Text>
					<Text class="text-sm text-slate-300">
						"A team chat and task board, live on every device."
					</Text>
				</View>
				{sign_in.screen()}
				{dev_sign_in()}
			</View>
		</ScrollView>
	}
}

/// Buttons that sign in as a teammate without a code: debug builds, in the
/// browser, where the server's dev sign-in sets the session cookie.
#[cfg(all(feature = "web", debug_assertions))]
#[allow(
	clippy::unnecessary_wraps,
	reason = "the same signature as the build without the buttons"
)]
fn dev_sign_in() -> Option<Element> {
	let client = leptoskit::query::use_query_client();
	let as_teammate = move |email: &'static str| {
		let client = client.clone();
		move || {
			let body = serde_json::json!({ "email": email }).to_string();
			let client = client.clone();
			any_spawner::Executor::spawn_local(async move {
				let signed_in = leptoskit::auth::client::send(
					leptoskit::auth::client::Method::Post,
					"/auth/dev/sign-in",
					Some(body),
				)
				.await;

				if signed_in.is_ok() {
					// The route set a new session cookie: the live connection
					// reconnects with it, and the session query reads it.
					leptoskit_rpc::client::reauthenticate();
					client
						.invalidate_scope(&leptoskit::auth::client::session())
						.await;
				}
			});
		}
	};

	Some(view! {
		<View class="items-center gap-2 p-4 rounded-xl bg-slate-900 w-full max-w-[420px]">
			<Text class="text-sm font-semibold">"Development"</Text>
			<Text class="text-xs text-slate-400 text-center">
				"Sign in as a teammate without a code. Open a second window to be two people."
			</Text>
			<View class="flex-row flex-wrap gap-2 justify-center">
				<Button
					class="rounded-lg bg-indigo-500 px-3 py-2 text-sm font-semibold"
					test_id="dev-ada"
					on:press=as_teammate("ada@huddle.dev")
				>
					"Ada"
				</Button>
				<Button
					class="rounded-lg bg-indigo-500 px-3 py-2 text-sm font-semibold"
					test_id="dev-grace"
					on:press=as_teammate("grace.hopper@huddle.dev")
				>
					"Grace"
				</Button>
				<Button
					class="rounded-lg bg-indigo-500 px-3 py-2 text-sm font-semibold"
					test_id="dev-lin"
					on:press=as_teammate("lin@huddle.dev")
				>
					"Lin"
				</Button>
			</View>
		</View>
	})
}

/// Native apps sign in with the emailed code: dev sign-in sets a browser
/// cookie, which a native app doesn't keep.
#[cfg(not(all(feature = "web", debug_assertions)))]
const fn dev_sign_in() -> Option<Element> {
	None
}
