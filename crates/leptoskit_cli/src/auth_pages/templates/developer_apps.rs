//! The developer portal: people register apps of their own that sign in
//! with yours, and replace or delete them. It needs the portal turned on,
//! with `developer_portal = true` in `[auth.idp]`.

use leptoskit::auth::ErrorResponse;
use leptoskit::idp::ClientAuth;
use leptoskit::idp::ClientInfo;
use leptoskit::idp::NewApp;
use leptoskit::idp::RegisteredApp;
use leptoskit::idp::client::delete_app;
use leptoskit::idp::client::my_apps;
use leptoskit::idp::client::register_app;
use leptoskit::idp::client::rotate_app_secret;
use leptoskit::idp::default_grant_types;
use leptoskit::leptos::prelude::Get;
use leptoskit::leptos::prelude::GetUntracked;
use leptoskit::leptos::prelude::RwSignal;
use leptoskit::leptos::prelude::Set;
use leptoskit::prelude::*;
use leptoskit::query::MutationHandle;
use leptoskit::query::QueryHandle;
use leptoskit::query::QueryObserverOptions;
use leptoskit::query::use_mutation;
use leptoskit::query::use_query_with;

use super::sign_in::sentence;

/// The developer portal's state. It's `Copy`, so handlers capture it freely.
#[derive(Clone, Copy)]
pub struct DeveloperApps {
	name: RwSignal<String>,
	redirect_uri: RwSignal<String>,
	scopes: RwSignal<String>,
	/// What's wrong with the form, found before anything is sent.
	problem: RwSignal<Option<String>>,
	/// Whether the screen has been shown, so an app without the portal
	/// never asks for it.
	opened: RwSignal<bool>,
	list: QueryHandle<Vec<ClientInfo>, ErrorResponse>,
	register: MutationHandle<NewApp, RegisteredApp, ErrorResponse>,
	rotate: MutationHandle<String, RegisteredApp, ErrorResponse>,
	delete: MutationHandle<String, bool, ErrorResponse>,
}

impl DeveloperApps {
	/// Creates the state in the current reactive owner, which needs a
	/// `QueryClient`. The apps are read once the screen is shown, while
	/// `enabled` says so, such as while someone is signed in.
	pub fn new(enabled: impl Fn() -> bool + Send + Sync + 'static) -> Self {
		let opened = RwSignal::new(false);

		Self {
			name: RwSignal::new(String::new()),
			redirect_uri: RwSignal::new(String::new()),
			scopes: RwSignal::new(String::new()),
			problem: RwSignal::new(None),
			opened,
			list: use_query_with(
				my_apps(),
				|| (),
				QueryObserverOptions::default().with_enabled(move || opened.get() && enabled()),
			),
			register: use_mutation(register_app()),
			rotate: use_mutation(rotate_app_secret()),
			delete: use_mutation(delete_app()),
		}
	}

	/// The whole screen: a secret just made, the person's apps, and the
	/// form that registers another.
	pub fn screen(&self) -> Element {
		if !self.opened.get_untracked() {
			self.opened.set(true);
		}

		let mut screen = View()
			.class("w-full max-w-[640px] gap-4 p-6")
			.child(Heading(1, "Your apps").class("text-2xl font-bold"))
			.child(
				Text(
					"Register an app of your own that signs people in with their account here. \
					 Each person decides what it may do.",
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

		// A secret is shown once, right after it's made.
		let secret = self
			.register
			.data()
			.or_else(|| self.rotate.data())
			.and_then(|made| Some((made.app.name, made.app.client_id, made.secret?)));

		if let Some((name, client_id, secret)) = secret {
			screen = screen.child(
				View()
					.class("gap-1 rounded-xl bg-amber-900 p-4")
					.child(Text(format!("{name}'s client ID: {client_id}")).class("text-sm"))
					.child(Text(format!("Its secret: {secret}")).class("font-semibold"))
					.child(
						Text("Copy the secret now: it won't be shown again.")
							.class("text-sm text-slate-300"),
					),
			);
		}

		let mut list = View().class("gap-2").aria_busy(self.list.is_fetching());

		for app in &apps {
			list = list.child(self.row(app));
		}

		let error = self
			.register
			.error()
			.or_else(|| self.rotate.error())
			.or_else(|| self.delete.error())
			.map(|error| sentence(&error.message));

		screen.child(list).child(self.form()).children(
			self.problem
				.get()
				.or(error)
				.map(|text| Alert(text).class("text-sm text-rose-300")),
		)
	}

	/// One app: its name, client ID and scopes, and its buttons.
	fn row(&self, app: &ClientInfo) -> Element {
		let busy = self.rotate.is_pending() || self.delete.is_pending();
		let mut actions = View().class("flex-row gap-2");

		if matches!(
			app.auth_method,
			ClientAuth::SecretBasic | ClientAuth::SecretPost
		) {
			let rotate = self.rotate;
			let client_id = app.client_id.clone();
			actions = actions.child(
				Button("New secret")
					.enabled(!busy)
					.aria_label(format!("New secret for {}", app.name))
					.on_press(move || rotate.mutate(client_id.clone()))
					.class("rounded-lg bg-slate-700 px-3 py-2 text-sm font-semibold"),
			);
		}

		let delete = self.delete;
		let client_id = app.client_id.clone();

		View()
			.class("gap-2 rounded-xl bg-slate-900 p-4")
			.child(Text(app.name.clone()).class("font-semibold"))
			.child(Text(format!("Client ID: {}", app.client_id)).class("text-sm text-slate-300"))
			.child(Text(app.scopes.join(" ")).class("text-sm text-slate-300"))
			.child(
				actions.child(
					Button("Delete")
						.enabled(!busy)
						.aria_label(format!("Delete {}", app.name))
						.on_press(move || delete.mutate(client_id.clone()))
						.class("rounded-lg bg-rose-700 px-3 py-2 text-sm font-semibold"),
				),
			)
	}

	/// The form that registers an app.
	fn form(&self) -> Element {
		let this = *self;
		let registering = self.register.is_pending();
		let field = |label: &'static str, placeholder: &'static str, value: RwSignal<String>| {
			TextInput()
				.aria_label(label)
				.placeholder(placeholder)
				.value(value.get_untracked())
				.on_change(move |text| value.set(text.to_owned()))
				.class("h-11 rounded-lg px-3 bg-white text-slate-900")
		};

		View()
			.class("gap-3 rounded-xl bg-slate-900 p-4")
			.child(Heading(2, "Register an app").class("text-lg font-semibold"))
			.child(field("App name", "My App", self.name))
			.child(field(
				"Redirect URI",
				"https://myapp.example/callback",
				self.redirect_uri,
			))
			.child(field("Scopes", "openid profile", self.scopes))
			.child(
				View()
					.class("flex-row gap-3")
					.child(
						Button("Register with a secret")
							.enabled(!registering)
							.on_press(move || this.submit(ClientAuth::SecretBasic))
							.class("flex-1 rounded-lg bg-indigo-500 px-4 py-3 font-semibold"),
					)
					.child(
						Button("Register a public app")
							.enabled(!registering)
							.on_press(move || this.submit(ClientAuth::None))
							.class("flex-1 rounded-lg bg-slate-700 px-4 py-3 font-semibold"),
					),
			)
	}

	/// The apps couldn't be read: the server is down, or the portal is off.
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

	fn submit(&self, auth_method: ClientAuth) {
		let name = self.name.get_untracked().trim().to_owned();
		let redirect_uri = self.redirect_uri.get_untracked().trim().to_owned();

		if name.is_empty() || redirect_uri.is_empty() {
			self.problem
				.set(Some("Give the app a name and a redirect URI.".into()));
			return;
		}

		self.problem.set(None);
		self.register.mutate(NewApp {
			name,
			redirect_uris: vec![redirect_uri],
			post_logout_redirect_uris: Vec::new(),
			scopes: self
				.scopes
				.get_untracked()
				.split_whitespace()
				.map(str::to_owned)
				.collect(),
			auth_method,
			grant_types: default_grant_types(),
			jwks: None,
		});
	}
}
