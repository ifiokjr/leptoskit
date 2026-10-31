//! Connecting a device: someone types the code a CLI or a TV shows, checks
//! it's the app they started, and lets it sign in from here, where they're
//! already signed in.

use leptoskit::auth::ErrorResponse;
use leptoskit::idp::DeviceDecision;
use leptoskit::idp::DeviceRequest;
use leptoskit::idp::client::decide_device;
use leptoskit::idp::client::device_request;
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

/// The device screen's state. It's `Copy`, so handlers capture it freely.
#[derive(Clone, Copy)]
pub struct DeviceCode {
	/// What's typed in the code field.
	typed: RwSignal<String>,
	/// The code being looked up, from the field or the page's `?user_code=`.
	user_code: RwSignal<Option<String>>,
	request: QueryHandle<DeviceRequest, ErrorResponse>,
	decide: MutationHandle<DeviceDecision, (), ErrorResponse>,
}

impl DeviceCode {
	/// Creates the state in the current reactive owner, which needs a
	/// `QueryClient`. A code is looked up only while `enabled` says so, such
	/// as while someone is signed in.
	pub fn new(enabled: impl Fn() -> bool + Send + Sync + 'static) -> Self {
		let user_code = RwSignal::new(None::<String>);

		Self {
			typed: RwSignal::new(String::new()),
			user_code,
			request: use_query_with(
				device_request(),
				move || user_code.get().unwrap_or_default(),
				QueryObserverOptions::default()
					.with_enabled(move || enabled() && user_code.get().is_some()),
			),
			decide: use_mutation(decide_device()),
		}
	}

	/// Looks up `user_code`, such as the `?user_code=` of the link a device
	/// shows. `None` asks for one.
	pub fn open(&self, user_code: Option<&str>) {
		let user_code = user_code.map(normalize).filter(|code| !code.is_empty());

		if self.user_code.get_untracked() != user_code {
			self.decide.reset();
			self.user_code.set(user_code);
		}
	}

	/// The whole screen: the code field, then the app and its two answers.
	pub fn screen(&self) -> Element {
		let step = match self.user_code.get() {
			None => self.enter_code(None),
			Some(_) if self.decide.is_success() => {
				let approved = self
					.decide
					.variables()
					.is_some_and(|decision| decision.approve);
				Status(if approved {
					"Done. You can go back to your device."
				} else {
					"Cancelled. Your device won't be signed in."
				})
				.class("text-slate-300")
			}
			Some(code) => {
				match (self.request.data(), self.request.error()) {
					(Some(request), _) => self.ask(&code, &request),
					(None, Some(error)) if error.error == "not_found" => {
						self.enter_code(Some(
							"That code didn't work. Check it, or start again on your device.",
						))
					}
					(None, Some(error)) => self.unreachable(&error),
					(None, None) => Status("Looking up the code…").class("text-slate-300"),
				}
			}
		};

		View()
			.class("min-h-screen items-center justify-center p-6 bg-slate-950 text-white")
			.child(
				View()
					.class("w-full max-w-[420px] gap-4 rounded-2xl bg-slate-900 p-6")
					.child(Heading(1, "Connect a device").class("text-2xl font-bold"))
					.child(step),
			)
	}

	/// The code field, with what was wrong with the last one, if anything.
	fn enter_code(&self, problem: Option<&'static str>) -> Element {
		let this = *self;
		let typed = self.typed;

		View()
			.class("gap-3")
			.child(Text("Enter the code your device shows.").class("text-sm text-slate-300"))
			.child(
				TextInput()
					.id("device-code")
					.aria_label("Device code")
					.aria_required(true)
					.placeholder("WDJB-MJHT")
					.value(typed.get_untracked())
					.on_change(move |text| typed.set(text.to_owned()))
					.class("h-11 rounded-lg px-3 bg-white text-slate-900"),
			)
			.children(problem.map(|text| Alert(text).class("text-sm text-rose-300")))
			.child(
				Button("Continue")
					.on_press(move || this.open(Some(&this.typed.get_untracked())))
					.class("rounded-lg bg-indigo-500 px-4 py-3 font-semibold"),
			)
	}

	/// The app the code belongs to, what it asks for, and the two answers.
	fn ask(&self, code: &str, request: &DeviceRequest) -> Element {
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
				Text(format!(
					"Check that your device shows {}, then let {} use your account.",
					request.user_code, request.app.name
				))
				.class("font-semibold"),
			)
			.child(Text("It will be able to:").class("text-sm font-semibold"))
			.child(scopes)
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
							.on_press(self.answer(code, false))
							.class("flex-1 rounded-lg bg-slate-700 px-4 py-3 font-semibold"),
					)
					.child(
						Button(if deciding { "Allowing…" } else { "Allow" })
							.enabled(!deciding)
							.aria_busy(deciding)
							.on_press(self.answer(code, true))
							.class("flex-1 rounded-lg bg-indigo-500 px-4 py-3 font-semibold"),
					),
			)
	}

	/// The code couldn't be looked up, such as when the server is down.
	fn unreachable(&self, error: &ErrorResponse) -> Element {
		let request = self.request;

		View()
			.class("gap-3")
			.child(Alert(sentence(&error.message)).class("text-sm text-rose-300"))
			.child(
				Button("Try again")
					.on_press(move || drop(request.refetch()))
					.class("rounded-lg bg-slate-700 px-4 py-3 font-semibold"),
			)
	}

	fn answer(&self, code: &str, approve: bool) -> impl Fn() + Send + Sync + 'static {
		let decide = self.decide;
		let user_code = code.to_owned();

		move || {
			decide.mutate(DeviceDecision {
				user_code: user_code.clone(),
				approve,
			});
		}
	}
}

/// A typed code as the server reads it: letters and the dash, in capitals,
/// without spaces or anything pasted along with it.
pub fn normalize(code: &str) -> String {
	code.chars()
		.filter(|c| c.is_ascii_alphanumeric() || *c == '-')
		.map(|c| c.to_ascii_uppercase())
		.collect()
}

#[cfg(test)]
mod tests {
	use super::normalize;

	#[test]
	fn codes_are_tidied_before_they_are_looked_up() {
		assert_eq!(normalize(" wdjb-mjht?<x> "), "WDJB-MJHTX");
		assert_eq!(normalize("  "), "");
	}
}
