//! "Connected apps": every app someone let sign in with their account, and
//! a way to disconnect each, which ends every token it holds.

use leptoskit::auth::ErrorResponse;
use leptoskit::idp::ConnectedApp;
use leptoskit::idp::client::connected_apps;
use leptoskit::idp::client::disconnect_app;
use leptoskit::prelude::*;
use leptoskit::query::MutationHandle;
use leptoskit::query::QueryHandle;
use leptoskit::query::QueryObserverOptions;
use leptoskit::query::use_mutation;
use leptoskit::query::use_query_with;

use super::devices::day;
use super::sign_in::sentence;

/// The connected apps screen's state. It's `Copy`, so handlers capture it
/// freely.
#[derive(Clone, Copy)]
pub struct ConnectedApps {
	list: QueryHandle<Vec<ConnectedApp>, ErrorResponse>,
	disconnect: MutationHandle<String, bool, ErrorResponse>,
}

impl ConnectedApps {
	/// Creates the state in the current reactive owner, which needs a
	/// `QueryClient`. The list is read only while `enabled` says so, such as
	/// while someone is signed in.
	pub fn new(enabled: impl Fn() -> bool + Send + Sync + 'static) -> Self {
		Self {
			list: use_query_with(
				connected_apps(),
				|| (),
				QueryObserverOptions::default().with_enabled(enabled),
			),
			disconnect: use_mutation(disconnect_app()),
		}
	}

	/// The whole screen: each app, and the button that disconnects it.
	pub fn screen(&self) -> Element {
		let mut screen = View()
			.class("w-full max-w-[640px] gap-4 p-6")
			.child(Heading(1, "Connected apps").class("text-2xl font-bold"))
			.child(
				Text(
					"These apps can sign in with your account. Disconnect any you no longer use: \
					 it's signed out straight away.",
				)
				.class("text-sm text-slate-300"),
			);

		let apps = match self.list.data() {
			Some(apps) => apps,
			None => {
				return screen.child(match self.list.error() {
					Some(error) => self.unreachable(&error),
					None => Status("Loading your apps…").class("text-slate-300"),
				});
			}
		};

		if apps.is_empty() {
			screen = screen.child(
				Text("No apps can use your account. Apps you sign in to with it appear here.")
					.class("text-sm text-slate-300"),
			);
		}

		let mut list = View().class("gap-2").aria_busy(self.list.is_fetching());

		for app in &apps {
			list = list.child(self.row(app));
		}

		screen.child(list).children(
			self.disconnect
				.error()
				.map(|error| Alert(sentence(&error.message)).class("text-sm text-rose-300")),
		)
	}

	/// One app: its name, what it may do, when it was last used, and its
	/// button.
	fn row(&self, app: &ConnectedApp) -> Element {
		let disconnect = self.disconnect;
		let client_id = app.app.client_id.clone();
		let busy = disconnect.is_pending();
		let scopes: Vec<&str> = app
			.scopes
			.iter()
			.map(|scope| {
				if scope.description.is_empty() {
					scope.name.as_str()
				} else {
					scope.description.as_str()
				}
			})
			.collect();

		View()
			.class("gap-2 rounded-xl bg-slate-900 p-4")
			.child(Text(app.app.name.clone()).class("font-semibold"))
			.child(Text(scopes.join(" ")).class("text-sm text-slate-300"))
			.child(
				Text(match app.last_used_at {
					Some(used) => format!("Last used {}", day(used)),
					None => format!("Connected {}", day(app.connected_at)),
				})
				.class("text-sm text-slate-300"),
			)
			.child(
				Button("Disconnect")
					.enabled(!busy)
					.aria_label(format!("Disconnect {}", app.app.name))
					.on_press(move || disconnect.mutate(client_id.clone()))
					.class("rounded-lg bg-rose-700 px-3 py-2 text-sm font-semibold"),
			)
	}

	/// The list couldn't be read, such as when the server is down.
	fn unreachable(&self, error: &ErrorResponse) -> Element {
		let list = self.list;

		View()
			.class("gap-3")
			.child(Alert(sentence(&error.message)).class("text-sm text-rose-300"))
			.child(
				Button("Try again")
					.on_press(move || drop(list.refetch()))
					.class("rounded-lg bg-slate-700 px-4 py-3 font-semibold"),
			)
	}
}
