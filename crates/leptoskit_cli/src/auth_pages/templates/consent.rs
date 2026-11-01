//! The consent screen: another app asks to sign someone in with their
//! account here, and to do some things for them. They say yes or no, and go
//! back to the app either way.

use leptoskit::auth::ErrorResponse;
use leptoskit::idp::ConsentDecision;
use leptoskit::idp::ConsentRedirect;
use leptoskit::idp::ConsentRequest;
use leptoskit::idp::client::consent;
use leptoskit::idp::client::decide_consent;
use leptoskit::leptos::prelude::Get;
use leptoskit::leptos::prelude::GetUntracked;
use leptoskit::leptos::prelude::RwSignal;
use leptoskit::leptos::prelude::Set;
use leptoskit::prelude::*;
use leptoskit::query::MutateCallbacks;
use leptoskit::query::MutationHandle;
use leptoskit::query::QueryHandle;
use leptoskit::query::QueryObserverOptions;
use leptoskit::query::SuccessArgs;
use leptoskit::query::use_mutation;
use leptoskit::query::use_query_with;

use super::Leave;
use super::sign_in::sentence;

/// The consent screen's state. It's `Copy`, so handlers capture it freely.
#[derive(Clone, Copy)]
pub struct Consent {
	/// The request the page's `?request=` names, once it's opened.
	request: RwSignal<Option<String>>,
	/// Set when the app hasn't said how to leave it.
	stuck: RwSignal<bool>,
	leave: Leave,
	details: QueryHandle<ConsentRequest, ErrorResponse>,
	decide: MutationHandle<ConsentDecision, ConsentRedirect, ErrorResponse>,
}

impl Consent {
	/// Creates the state in the current reactive owner, which needs a
	/// `QueryClient`. The request is read once [`open`](Self::open) names it,
	/// while `enabled` says so, such as while someone is signed in.
	pub fn new(enabled: impl Fn() -> bool + Send + Sync + 'static, leave: Leave) -> Self {
		let request = RwSignal::new(None::<String>);

		Self {
			request,
			stuck: RwSignal::new(false),
			leave,
			details: use_query_with(
				consent(),
				move || request.get().unwrap_or_default(),
				QueryObserverOptions::default()
					.with_enabled(move || enabled() && request.get().is_some()),
			),
			decide: use_mutation(decide_consent()),
		}
	}

	/// Shows the request `request` names: the `?request=` that
	/// `/oauth/authorize` sent the browser here with.
	pub fn open(&self, request: &str) {
		if self.request.get_untracked().as_deref() != Some(request) {
			self.decide.reset();
			self.request.set(Some(request.to_owned()));
		}
	}

	/// The whole screen: what the app asks for, and the two answers.
	pub fn screen(&self) -> Element {
		let step = if self.request.get().is_none() {
			Alert("This page needs the request an app sent you here with.")
				.class("text-sm text-rose-300")
		} else if self.stuck.get() {
			Alert(
				"This app hasn't said how to go back. Call `auth::screens().navigate_with(…)` \
				 when it starts.",
			)
			.class("text-sm text-rose-300")
		} else {
			match (self.details.data(), self.details.error()) {
				(Some(request), _) => self.ask(&request),

				(None, Some(error)) if error.error == "not_found" => {
					Alert(
						"This request has expired or was already answered. Go back to the app and \
						 start again.",
					)
					.class("text-sm text-rose-300")
				}

				(None, Some(error)) => self.unreachable(&error),
				(None, None) => Status("Loading…").class("text-slate-300"),
			}
		};

		View()
			.class("min-h-screen items-center justify-center p-6 bg-slate-950 text-white")
			.child(
				View()
					.class("w-full max-w-[420px] gap-4 rounded-2xl bg-slate-900 p-6")
					.child(step),
			)
	}

	/// The app, what it asks for, and where the person goes back to.
	fn ask(&self, request: &ConsentRequest) -> Element {
		let this = *self;
		let deciding = self.decide.is_pending();
		let mut scopes = View().class("gap-2");

		for scope in &request.scopes {
			let text = if scope.description.is_empty() {
				scope.name.clone()
			} else {
				scope.description.clone()
			};

			scopes = scopes.child(Text(format!("• {text}")).class("text-sm text-slate-300"));
		}

		View()
			.class("gap-3")
			.child(
				Heading(1, format!("{} wants to use your account", request.app.name))
					.class("text-2xl font-bold"),
			)
			.child(Text("It will be able to:").class("text-sm font-semibold"))
			.child(scopes)
			.child(
				Text(format!(
					"Either way, you'll go back to {}.",
					request.redirect_host
				))
				.class("text-sm text-slate-300"),
			)
			.children(
				self.decide
					.error()
					.map(|error| Alert(sentence(&error.message)).class("text-sm text-rose-300")),
			)
			.child(
				View()
					.class("flex-row gap-3")
					.child(
						Button("Cancel")
							.enabled(!deciding)
							.on_press(move || this.answer(false))
							.class("flex-1 rounded-lg bg-slate-700 px-4 py-3 font-semibold"),
					)
					.child(
						Button(if deciding { "Allowing…" } else { "Allow" })
							.enabled(!deciding)
							.aria_busy(deciding)
							.on_press(move || this.answer(true))
							.class("flex-1 rounded-lg bg-indigo-500 px-4 py-3 font-semibold"),
					),
			)
	}

	/// The request couldn't be read, such as when the server is down.
	fn unreachable(&self, error: &ErrorResponse) -> Element {
		let details = self.details;

		View()
			.class("gap-3")
			.child(Alert(sentence(&error.message)).class("text-sm text-rose-300"))
			.child(
				Button("Try again")
					.on_press(move || drop(details.refetch()))
					.class("rounded-lg bg-slate-700 px-4 py-3 font-semibold"),
			)
	}

	/// Answers, then goes back to the app, with a code or with the refusal.
	fn answer(&self, approve: bool) {
		let Some(request) = self.request.get_untracked() else {
			return;
		};
		let this = *self;

		self.decide.mutate_with(
			ConsentDecision { request, approve },
			MutateCallbacks::new().on_success(
				move |answered: SuccessArgs<ConsentRedirect, ConsentDecision, ()>| {
					if !this.leave.to(&answered.data.redirect_to) {
						this.stuck.set(true);
					}
				},
			),
		);
	}
}
