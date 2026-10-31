//! "Security": the ways to sign in and to prove it's you. Passkeys, an
//! authenticator app with the recovery codes that stand in for it when the
//! phone is lost, and connected accounts such as Google, which can also
//! bring another account's sign-in methods into this one.
//!
//! Changing these asks the person to prove it's them again. With a passkey
//! or an authenticator on, the screen asks for one and then tries the change
//! again; without either, signing in again is the only proof left.

use leptoskit::auth::ErrorResponse;
use leptoskit::auth::Factor;
use leptoskit::auth::Identity;
use leptoskit::auth::IdentityInfo;
use leptoskit::auth::MergeOffer;
use leptoskit::auth::MfaStatus;
use leptoskit::auth::Passkey;
use leptoskit::auth::PasskeyInfo;
use leptoskit::auth::ProviderInfo;
use leptoskit::auth::ProviderOutcome;
use leptoskit::auth::PublicSession;
use leptoskit::auth::RecoveryCodes;
use leptoskit::auth::StartProvider;
use leptoskit::auth::TotpSetup;
use leptoskit::auth::client::MergeInput;
use leptoskit::auth::client::PasskeyAdded;
use leptoskit::auth::client::TotpConfirmation;
use leptoskit::auth::client::add_passkey;
use leptoskit::auth::client::cancel_merge;
use leptoskit::auth::client::confirm_totp;
use leptoskit::auth::client::disable_totp;
use leptoskit::auth::client::enroll_totp;
use leptoskit::auth::client::identities;
use leptoskit::auth::client::link_provider;
use leptoskit::auth::client::merge_accounts;
use leptoskit::auth::client::merge_offer;
use leptoskit::auth::client::mfa;
use leptoskit::auth::client::passkeys;
use leptoskit::auth::client::provider_outcome;
use leptoskit::auth::client::qr_code;
use leptoskit::auth::client::regenerate_recovery_codes;
use leptoskit::auth::client::remove_passkey;
use leptoskit::auth::client::sign_out_mutation;
use leptoskit::auth::client::step_up;
use leptoskit::auth::client::step_up_with_passkey;
use leptoskit::auth::client::unlink_identity;
use leptoskit::auth::client::use_providers;
use leptoskit::db::Id;
use leptoskit::leptos::prelude::Effect;
use leptoskit::leptos::prelude::Get;
use leptoskit::leptos::prelude::GetUntracked;
use leptoskit::leptos::prelude::RwSignal;
use leptoskit::leptos::prelude::Set;
use leptoskit::native::Color;
use leptoskit::native::PathData;
use leptoskit::native::VectorPath;
use leptoskit::native::ViewBox;
use leptoskit::prelude::*;
use leptoskit::query::ErrorArgs;
use leptoskit::query::MutateCallbacks;
use leptoskit::query::MutationHandle;
use leptoskit::query::QueryHandle;
use leptoskit::query::QueryObserverOptions;
use leptoskit::query::SuccessArgs;
use leptoskit::query::use_mutation;
use leptoskit::query::use_query_with;

use super::devices::day;
use super::sign_in::passkey_problem;
use super::sign_in::provider_failure;
use super::sign_in::sentence;

/// A change that asked for step-up, to try again once it's done.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
	/// Turning an authenticator app on.
	Enroll,
	/// New recovery codes.
	Regenerate,
	/// Turning the authenticator app off.
	Disable,
	/// Adding a passkey.
	AddPasskey,
	/// Removing a passkey.
	RemovePasskey(Id<Passkey>),
	/// Connecting a provider.
	Link(String),
	/// Disconnecting a provider.
	Unlink(Id<Identity>),
	/// Merging the other account in.
	Merge,
}

/// The security screen's state. It's `Copy`, so handlers capture it freely.
#[derive(Clone, Copy)]
pub struct Security {
	status: QueryHandle<MfaStatus, ErrorResponse>,
	passkeys: QueryHandle<Vec<PasskeyInfo>, ErrorResponse>,
	identities: QueryHandle<Vec<IdentityInfo>, ErrorResponse>,
	providers: QueryHandle<Vec<ProviderInfo>, ErrorResponse>,
	offer: QueryHandle<Option<MergeOffer>, ErrorResponse>,
	enroll: MutationHandle<(), TotpSetup, ErrorResponse>,
	confirm: MutationHandle<String, TotpConfirmation, ErrorResponse>,
	disable: MutationHandle<(), (), ErrorResponse>,
	regenerate: MutationHandle<(), RecoveryCodes, ErrorResponse>,
	add_passkey: MutationHandle<Option<String>, PasskeyAdded, ErrorResponse>,
	remove_passkey: MutationHandle<Id<Passkey>, (), ErrorResponse>,
	link: MutationHandle<StartProvider, (), ErrorResponse>,
	unlink: MutationHandle<Id<Identity>, (), ErrorResponse>,
	merge: MutationHandle<MergeInput, (), ErrorResponse>,
	cancel_merge: MutationHandle<(), (), ErrorResponse>,
	step_up: MutationHandle<String, PublicSession, ErrorResponse>,
	step_up_passkey: MutationHandle<(), PublicSession, ErrorResponse>,
	sign_out: MutationHandle<(), (), ErrorResponse>,
	/// The code typed to turn the authenticator on, to step up, or for the
	/// other account in a merge.
	code: RwSignal<String>,
	/// Recovery codes to show once, until the person says they've saved
	/// them.
	codes: RwSignal<Option<Vec<String>>>,
	/// The change waiting for step-up.
	waiting: RwSignal<Option<Change>>,
	/// What the server last refused, in words.
	problem: RwSignal<Option<String>>,
	/// What a provider that just sent the person back said.
	notice: RwSignal<Option<String>>,
	/// Whether the person must sign in again before changing anything.
	reauthenticate: RwSignal<bool>,
}

impl Security {
	/// Creates the state in the current reactive owner, which needs a
	/// `QueryClient`. The queries run only while `enabled` says so, such as
	/// while someone is signed in.
	pub fn new(enabled: impl Fn() -> bool + Clone + Send + Sync + 'static) -> Self {
		let notice = RwSignal::new(None);
		// Coming back from connecting a provider, say how it went.
		Effect::new(move |_| {
			notice.set(match provider_outcome() {
				Some(ProviderOutcome::Linked) => Some("It's connected.".to_owned()),
				Some(ProviderOutcome::AlreadyLinked) => {
					Some("That account was already connected.".to_owned())
				}
				Some(ProviderOutcome::Failed(code)) => Some(provider_failure(&code)),
				_ => None,
			});
		});

		Self {
			status: use_query_with(
				mfa(),
				|| (),
				QueryObserverOptions::default().with_enabled(enabled.clone()),
			),
			passkeys: use_query_with(
				passkeys(),
				|| (),
				QueryObserverOptions::default().with_enabled(enabled.clone()),
			),
			identities: use_query_with(
				identities(),
				|| (),
				QueryObserverOptions::default().with_enabled(enabled.clone()),
			),
			providers: use_providers(),
			offer: use_query_with(
				merge_offer(),
				|| (),
				QueryObserverOptions::default().with_enabled(enabled.clone()),
			),
			enroll: use_mutation(enroll_totp()),
			confirm: use_mutation(confirm_totp()),
			disable: use_mutation(disable_totp()),
			regenerate: use_mutation(regenerate_recovery_codes()),
			add_passkey: use_mutation(add_passkey()),
			remove_passkey: use_mutation(remove_passkey()),
			link: use_mutation(link_provider()),
			unlink: use_mutation(unlink_identity()),
			merge: use_mutation(merge_accounts()),
			cancel_merge: use_mutation(cancel_merge()),
			step_up: use_mutation(step_up()),
			step_up_passkey: use_mutation(step_up_with_passkey()),
			sign_out: use_mutation(sign_out_mutation()),
			code: RwSignal::new(String::new()),
			codes: RwSignal::new(None),
			waiting: RwSignal::new(None),
			problem: RwSignal::new(None),
			notice,
			reauthenticate: RwSignal::new(false),
		}
	}

	/// The whole screen.
	pub fn screen(&self) -> Element {
		let screen = View()
			.class("w-full max-w-[640px] gap-4 p-6")
			.child(Heading(1, "Security").class("text-2xl font-bold"))
			.children(
				self.notice
					.get()
					.map(|notice| Status(notice).class("text-sm text-emerald-300")),
			);

		if let Some(codes) = self.codes.get() {
			return screen.child(self.recovery_codes(&codes));
		}

		if self.reauthenticate.get() {
			return screen.child(self.sign_in_again());
		}

		let status = match self.status.data() {
			Some(status) => status,
			None => {
				return screen.child(match self.status.error() {
					Some(error) => Alert(sentence(&error.message)).class("text-sm text-rose-300"),
					None => Status("Loading…").class("text-slate-300"),
				});
			}
		};

		if self.waiting.get().is_some() {
			return screen.child(self.confirm_its_you(&status));
		}

		if let Some(offer) = self.offer.data().flatten() {
			return screen.child(self.merge_offer(&offer));
		}

		let authenticator = if status.totp {
			self.turned_on(&status)
		} else {
			match self.enroll.data() {
				Some(setup) => self.setting_up(&setup),
				None => self.turned_off(),
			}
		};
		let required = status.required && !status.enrolled();

		screen
			.children(required.then(|| {
				Alert(
					"Your account must use two-step verification. Add a passkey or an \
					 authenticator app to continue.",
				)
				.class("text-sm text-amber-300")
			}))
			.child(self.passkeys())
			.child(Heading(2, "Authenticator app").class("text-lg font-semibold"))
			.child(authenticator)
			.child(self.connected_accounts())
			.children(
				self.problem
					.get()
					.map(|problem| Alert(problem).class("text-sm text-rose-300")),
			)
	}

	/// The account's passkeys, and adding one.
	fn passkeys(&self) -> Element {
		let this = *self;
		let adding = self.add_passkey.is_pending();
		let mut list = View()
			.class("gap-3")
			.child(Heading(2, "Passkeys").class("text-lg font-semibold"))
			.child(
				Text(
					"A passkey signs you in with your fingerprint, face, or screen lock, and only \
					 works on this site, so it can't be phished.",
				)
				.class("text-sm text-slate-300"),
			);

		for passkey in self.passkeys.data().unwrap_or_default() {
			let id = passkey.id;
			let used = passkey.last_used_at.map_or_else(
				|| "Not used yet".to_owned(),
				|at| format!("Last used {}", day(at)),
			);
			let synced = if passkey.backed_up {
				"Synced to your other devices"
			} else {
				"On one device"
			};
			list = list.child(
				View()
					.class("flex-row items-center gap-3 rounded-lg bg-slate-800 p-3")
					.child(
						View()
							.class("flex-1 gap-1")
							.child(Text(passkey.name.clone()).class("font-semibold"))
							.child(
								Text(format!("{synced}. {used}.")).class("text-xs text-slate-400"),
							),
					)
					.child(
						Button("Remove")
							.enabled(!self.remove_passkey.is_pending())
							.aria_label(format!("Remove {}", passkey.name))
							.on_press(move || this.run(Change::RemovePasskey(id)))
							.class("rounded-lg bg-slate-700 px-3 py-2 text-sm font-semibold"),
					),
			);
		}

		list.child(
			Button(if adding {
				"Waiting for your passkey…"
			} else {
				"Add a passkey"
			})
			.enabled(!adding)
			.aria_busy(adding)
			.on_press(move || this.run(Change::AddPasskey))
			.class("rounded-lg bg-indigo-500 px-4 py-3 font-semibold"),
		)
	}

	/// No authenticator yet.
	fn turned_off(&self) -> Element {
		let this = *self;
		let busy = self.enroll.is_pending();

		View()
			.class("gap-3")
			.child(Status("No authenticator app is set up.").class("text-slate-300"))
			.child(
				Button(if busy {
					"Starting…"
				} else {
					"Set up an authenticator app"
				})
				.enabled(!busy)
				.aria_busy(busy)
				.on_press(move || this.run(Change::Enroll))
				.class("rounded-lg bg-slate-700 px-4 py-3 font-semibold"),
			)
	}

	/// The code to scan or key to type into the app, and the code that
	/// proves it was added.
	fn setting_up(&self, setup: &TotpSetup) -> Element {
		let this = *self;
		let code = self.code;
		let confirming = self.confirm.is_pending();

		View()
			.class("gap-3")
			.child(
				Text(
					"Scan this code with your authenticator app, or add an account and enter the \
					 key below it.",
				)
				.class("text-sm text-slate-300"),
			)
			.children(qr(&setup.uri))
			.child(
				Text(group(&setup.secret))
					.aria_label("Setup key")
					.class("rounded-lg bg-slate-800 p-3 text-lg font-semibold"),
			)
			.child(Text("Code from the app").class("text-sm font-semibold"))
			.child(
				TextInput()
					.id("security-totp-code")
					.aria_label("Code from the app")
					.aria_required(true)
					.placeholder("123456")
					.value(code.get_untracked())
					.on_change(move |text| code.set(text.to_owned()))
					.class("h-11 rounded-lg px-3 bg-white text-slate-900"),
			)
			.child(
				Button(if confirming {
					"Turning on…"
				} else {
					"Turn on"
				})
				.enabled(!confirming)
				.aria_busy(confirming)
				.on_press(move || this.turn_on())
				.class("rounded-lg bg-indigo-500 px-4 py-3 font-semibold"),
			)
	}

	/// An authenticator is on: how many recovery codes are left, and the
	/// changes.
	fn turned_on(&self, status: &MfaStatus) -> Element {
		let this = *self;
		let busy = self.regenerate.is_pending() || self.disable.is_pending();
		let left = match status.recovery_codes {
			0 => {
				"You have no recovery codes left. Make new ones now, in case you lose your phone."
					.to_owned()
			}
			1 => "You have 1 recovery code left.".to_owned(),
			left => format!("You have {left} recovery codes left."),
		};

		View()
			.class("gap-3")
			.child(Status("An authenticator app is on.").class("text-emerald-300"))
			.child(Text(left).class("text-sm text-slate-300"))
			.child(
				Button("Make new recovery codes")
					.enabled(!busy)
					.aria_description("The old ones stop working")
					.on_press(move || this.run(Change::Regenerate))
					.class("rounded-lg bg-slate-700 px-4 py-3 font-semibold"),
			)
			.child(
				Button("Turn off the authenticator app")
					.enabled(!busy)
					.aria_description("Also signs out every other device")
					.on_press(move || this.run(Change::Disable))
					.class("rounded-lg bg-rose-600 px-4 py-3 font-semibold"),
			)
	}

	/// Connected accounts, and the providers that could be.
	fn connected_accounts(&self) -> Element {
		let this = *self;
		let identities = self.identities.data().unwrap_or_default();
		let providers = self.providers.data().unwrap_or_default();

		if identities.is_empty() && providers.is_empty() {
			return View();
		}

		let mut section = View()
			.class("gap-3")
			.child(Heading(2, "Connected accounts").class("text-lg font-semibold"));

		for identity in &identities {
			let id = identity.id;
			let who = identity
				.name
				.clone()
				.or_else(|| identity.email.clone())
				.unwrap_or_default();
			section = section.child(
				View()
					.class("flex-row items-center gap-3 rounded-lg bg-slate-800 p-3")
					.child(
						View()
							.class("flex-1 gap-1")
							.child(Text(identity.provider_name.clone()).class("font-semibold"))
							.child(Text(who).class("text-xs text-slate-400")),
					)
					.child(
						Button("Disconnect")
							.enabled(!self.unlink.is_pending())
							.aria_label(format!("Disconnect {}", identity.provider_name))
							.on_press(move || this.run(Change::Unlink(id)))
							.class("rounded-lg bg-slate-700 px-3 py-2 text-sm font-semibold"),
					),
			);
		}

		for provider in providers {
			if identities
				.iter()
				.any(|identity| identity.provider == provider.id)
			{
				continue;
			}

			let id = provider.id.clone();
			section = section.child(
				Button(format!("Connect {}", provider.name))
					.enabled(!self.link.is_pending())
					.on_press(move || this.run(Change::Link(id.clone())))
					.class("rounded-lg bg-slate-700 px-4 py-3 font-semibold"),
			);
		}

		section
	}

	/// Connecting a provider found another account: merge it in?
	fn merge_offer(&self, offer: &MergeOffer) -> Element {
		let this = *self;
		let code = self.code;
		let merging = self.merge.is_pending();
		let needs_code = offer.second_factor.contains(&Factor::Totp)
			|| offer.second_factor.contains(&Factor::RecoveryCode);
		let needs_passkey = offer.second_factor.contains(&Factor::Passkey);
		let mut moves = offer.emails.clone();
		moves.extend(offer.identities.iter().cloned());

		if offer.passkeys > 0 {
			moves.push(format!(
				"{} passkey{}",
				offer.passkeys,
				if offer.passkeys == 1 { "" } else { "s" }
			));
		}

		View()
			.class("gap-3 rounded-lg bg-slate-800 p-4")
			.child(
				Heading(2, "That login already has its own account").class("text-lg font-semibold"),
			)
			.child(
				Text(format!(
					"It was created on {}. Merging moves its sign-in methods here ({}) and \
					 deletes its data. This can't be undone.",
					day(offer.created_at),
					moves.join(", ")
				))
				.class("text-sm text-slate-300"),
			)
			.children(needs_code.then(|| {
				TextInput()
					.id("security-merge-code")
					.aria_label("A code from the other account's authenticator app")
					.placeholder("123456")
					.value(code.get_untracked())
					.on_change(move |text| code.set(text.to_owned()))
					.class("h-11 rounded-lg px-3 bg-white text-slate-900")
			}))
			.children(
				self.problem
					.get()
					.map(|problem| Alert(problem).class("text-sm text-rose-300")),
			)
			.child(
				View()
					.class("flex-row gap-3")
					.child(
						Button("Cancel")
							.on_press(move || {
								this.problem.set(None);
								this.cancel_merge.mutate(());
							})
							.class("flex-1 rounded-lg bg-slate-700 px-4 py-3 font-semibold"),
					)
					.children(needs_passkey.then(|| {
						Button("Use its passkey")
							.enabled(!merging)
							.on_press(move || this.merge_with(true))
							.class("flex-1 rounded-lg bg-slate-700 px-4 py-3 font-semibold")
					}))
					.child(
						Button(if merging {
							"Merging…"
						} else {
							"Merge accounts"
						})
						.enabled(!merging)
						.aria_busy(merging)
						.on_press(move || this.merge_with(false))
						.class("flex-1 rounded-lg bg-rose-600 px-4 py-3 font-semibold"),
					),
			)
	}

	/// New recovery codes, shown once.
	fn recovery_codes(&self, codes: &[String]) -> Element {
		let shown = self.codes;
		let mut list = View().class("gap-1 rounded-lg bg-slate-800 p-3");

		for code in codes {
			list = list.child(Text(code.clone()).class("text-lg font-semibold"));
		}

		View()
			.class("gap-3")
			.child(
				Status(
					"Save these recovery codes somewhere safe, such as a password manager. Each \
					 one signs you in once if you lose your phone or passkey. You won't see them \
					 again.",
				)
				.class("text-sm text-slate-300"),
			)
			.child(list)
			.child(
				Button("I've saved them")
					.on_press(move || shown.set(None))
					.class("rounded-lg bg-indigo-500 px-4 py-3 font-semibold"),
			)
	}

	/// Step-up: a passkey, or a code from the app, before the change goes
	/// through.
	fn confirm_its_you(&self, status: &MfaStatus) -> Element {
		let this = *self;
		let code = self.code;
		let checking = self.step_up.is_pending() || self.step_up_passkey.is_pending();
		let mut confirm = View()
			.class("gap-3")
			.child(Status("Confirm it's you before this change.").class("text-sm text-slate-300"));

		if status.passkeys > 0 {
			confirm = confirm.child(
				Button("Use a passkey")
					.enabled(!checking)
					.on_press(move || this.confirm_with_passkey())
					.class("rounded-lg bg-indigo-500 px-4 py-3 font-semibold"),
			);
		}

		if status.totp || status.recovery_codes > 0 {
			confirm = confirm
				.child(
					TextInput()
						.id("security-step-up-code")
						.aria_label("Authenticator or recovery code")
						.aria_required(true)
						.placeholder("123456")
						.value(code.get_untracked())
						.on_change(move |text| code.set(text.to_owned()))
						.class("h-11 rounded-lg px-3 bg-white text-slate-900"),
				)
				.child(
					Button(if checking {
						"Checking…"
					} else {
						"Confirm with the code"
					})
					.enabled(!checking)
					.aria_busy(checking)
					.on_press(move || this.confirm_step_up())
					.class("rounded-lg bg-slate-700 px-4 py-3 font-semibold"),
				);
		}

		confirm
			.children(
				self.problem
					.get()
					.map(|problem| Alert(problem).class("text-sm text-rose-300")),
			)
			.child(
				Button("Cancel")
					.on_press(move || this.cancel())
					.class("rounded-lg px-4 py-2 text-sm text-indigo-300"),
			)
	}

	/// Without a second factor, a recent sign-in is the only proof.
	fn sign_in_again(&self) -> Element {
		let this = *self;

		View()
			.class("gap-3")
			.child(
				Alert("For your security, sign in again before you change this.")
					.class("text-sm text-amber-300"),
			)
			.child(
				Button("Sign out to sign in again")
					.on_press(move || {
						let reauthenticate = this.reauthenticate;
						this.sign_out.mutate_with(
							(),
							MutateCallbacks::new().on_success(move |_| reauthenticate.set(false)),
						);
					})
					.class("rounded-lg bg-indigo-500 px-4 py-3 font-semibold"),
			)
	}

	/// Runs `change`, asking for step-up first if the server wants it.
	fn run(&self, change: Change) {
		let this = *self;
		self.problem.set(None);
		self.notice.set(None);
		let failed = {
			let change = change.clone();
			move |error: &ErrorResponse| this.refused(change.clone(), error)
		};

		match change {
			Change::Enroll => {
				self.enroll.mutate_with(
					(),
					MutateCallbacks::new().on_error(move |args| failed(&args.error)),
				);
			}
			Change::Regenerate => {
				self.regenerate.mutate_with(
					(),
					MutateCallbacks::new()
						.on_success(move |args: SuccessArgs<RecoveryCodes, (), ()>| {
							this.codes.set(Some(args.data.codes));
						})
						.on_error(move |args| failed(&args.error)),
				);
			}
			Change::Disable => {
				self.disable.mutate_with(
					(),
					MutateCallbacks::new().on_error(move |args| failed(&args.error)),
				);
			}
			Change::AddPasskey => {
				self.add_passkey.mutate_with(
					None,
					MutateCallbacks::new()
						.on_success(move |args: SuccessArgs<PasskeyAdded, Option<String>, ()>| {
							this.codes
								.set(args.data.recovery_codes.map(|codes| codes.codes));
						})
						.on_error(move |args| failed(&args.error)),
				);
			}
			Change::RemovePasskey(id) => {
				self.remove_passkey.mutate_with(
					id,
					MutateCallbacks::new().on_error(move |args| failed(&args.error)),
				);
			}
			Change::Link(provider) => {
				self.link.mutate_with(
					StartProvider {
						provider,
						redirect_to: None,
					},
					MutateCallbacks::new().on_error(move |args| failed(&args.error)),
				);
			}
			Change::Unlink(id) => {
				self.unlink.mutate_with(
					id,
					MutateCallbacks::new().on_error(move |args| failed(&args.error)),
				);
			}
			Change::Merge => self.merge_with(false),
		}
	}

	/// Merges the other account in, with its code or passkey when it has
	/// two-step verification.
	fn merge_with(&self, passkey: bool) {
		let this = *self;
		let code = self.code.get_untracked().trim().to_owned();
		self.problem.set(None);
		self.merge.mutate_with(
			MergeInput {
				mode: None,
				confirm: true,
				code: (!code.is_empty()).then_some(code),
				passkey,
			},
			MutateCallbacks::new()
				.on_success(move |_| {
					this.code.set(String::new());
					this.notice.set(Some("The accounts are merged.".to_owned()));
				})
				.on_error(move |args: ErrorArgs<ErrorResponse, MergeInput, ()>| {
					match args.error.error.as_str() {
						"step_up_required"
							if this
								.offer
								.data()
								.flatten()
								.is_some_and(|offer| !offer.second_factor.is_empty())
								&& !passkey
								&& args.variables.code.is_none() =>
						{
							this.problem.set(Some(
								"Enter a code from the other account's authenticator app, or use \
								 its passkey."
									.into(),
							));
						}
						_ => this.refused(Change::Merge, &args.error),
					}
				}),
		);
	}

	fn turn_on(&self) {
		let code = self.code.get_untracked().trim().to_owned();

		if code.is_empty() {
			self.problem
				.set(Some("Enter the code your app shows.".into()));
			return;
		}

		let this = *self;
		self.problem.set(None);
		self.confirm.mutate_with(
			code,
			MutateCallbacks::new()
				.on_success(move |args: SuccessArgs<TotpConfirmation, String, ()>| {
					this.code.set(String::new());
					this.enroll.reset();
					this.codes
						.set(args.data.recovery_codes.map(|codes| codes.codes));
				})
				.on_error(move |args| this.refused(Change::Enroll, &args.error)),
		);
	}

	fn confirm_step_up(&self) {
		let code = self.code.get_untracked().trim().to_owned();

		if code.is_empty() {
			self.problem
				.set(Some("Enter a code to confirm it's you.".into()));
			return;
		}

		let this = *self;
		self.problem.set(None);
		self.step_up.mutate_with(
			code,
			MutateCallbacks::new()
				.on_success(move |_| this.stepped_up())
				.on_error(move |args: ErrorArgs<ErrorResponse, String, ()>| {
					this.problem.set(Some(sentence(&args.error.message)));
				}),
		);
	}

	fn confirm_with_passkey(&self) {
		let this = *self;
		self.problem.set(None);
		self.step_up_passkey.mutate_with(
			(),
			MutateCallbacks::new()
				.on_success(move |_| this.stepped_up())
				.on_error(move |args: ErrorArgs<ErrorResponse, (), ()>| {
					this.problem.set(Some(passkey_problem(&args.error)));
				}),
		);
	}

	/// Step-up worked: try the change that asked for it again.
	fn stepped_up(&self) {
		self.code.set(String::new());

		if let Some(change) = self.waiting.get_untracked() {
			self.waiting.set(None);
			self.run(change);
		}
	}

	fn cancel(&self) {
		self.waiting.set(None);
		self.code.set(String::new());
		self.problem.set(None);
	}

	/// What to do when the server refuses `change`.
	fn refused(&self, change: Change, error: &ErrorResponse) {
		match error.error.as_str() {
			"step_up_required" => self.waiting.set(Some(change)),
			"reauthenticate" => self.reauthenticate.set(true),
			"last_sign_in_method" => {
				self.problem.set(Some(
					"Keep at least one way to sign in, and a second factor if your account needs \
					 one."
						.into(),
				));
			}
			_ => self.problem.set(Some(passkey_problem(error))),
		}
	}
}

/// The setup link as a QR code, black on white, which every platform
/// draws. `None` when it can't be one; the key below it still works.
fn qr(uri: &str) -> Option<Element> {
	let code = qr_code(uri)?;
	let side = code.side() as f32;
	let view_box = ViewBox::new(0.0, 0.0, side, side).ok()?;
	let path = PathData::parse(&code.path()).ok()?;
	Some(
		Vector(view_box)
			.aria_label("QR code for your authenticator app")
			.class("w-48 h-48 bg-white")
			.path(VectorPath::new(path).fill(Color::rgba(0, 0, 0, 255))),
	)
}

/// A setup key in groups of four, as authenticator apps print them, so it's
/// easier to type.
pub fn group(key: &str) -> String {
	key.as_bytes()
		.chunks(4)
		.map(|chunk| String::from_utf8_lossy(chunk).into_owned())
		.collect::<Vec<_>>()
		.join(" ")
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn keys_are_grouped_in_fours() {
		assert_eq!(group("GEZDGNBVGY3TQOJQ"), "GEZD GNBV GY3T QOJQ");
		assert_eq!(group("ABCDEF"), "ABCD EF");
		assert_eq!(group(""), "");
	}

	#[test]
	fn a_setup_link_draws_as_a_code() {
		assert!(qr("otpauth://totp/Notes:ada%40example.com?issuer=Notes").is_some());
		assert!(qr(&"x".repeat(4000)).is_none());
	}
}
