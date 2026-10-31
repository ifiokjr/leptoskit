//! Connected accounts: what a third-party sign-in does once the provider
//! has said who someone is, and linking, unlinking, and merging accounts.
//!
//! Identity is the provider and its subject, never the email address. A new
//! subject always makes a new account, or attaches to the signed-in one when
//! it's connecting; an address that's already someone else's is never
//! enough to sign in to their account. Two accounts become one only when the
//! person proves they control both, and confirms.

use std::fmt;

use leptoskit_db::Connection;
use leptoskit_db::Email;
use leptoskit_db::Id;
use leptoskit_db::Tx;
use serde::Deserialize;
use serde::Serialize;

use crate::Aal;
use crate::Account;
use crate::Auth;
use crate::AuthError;
use crate::EmailAddress;
use crate::Event;
use crate::EventKind;
use crate::Identity;
use crate::IdentityInfo;
use crate::MergeAccounts;
use crate::MergeMode;
use crate::MergeOffer;
use crate::MergedAccount;
use crate::NewAccount;
use crate::NewEmailAddress;
use crate::NewIdentity;
use crate::NewMergedAccount;
use crate::Passkey;
use crate::Profile;
use crate::ProviderOutcome;
use crate::Purpose;
use crate::RequestInfo;
use crate::Result;
use crate::Session;
use crate::SessionToken;
use crate::SignIn;
use crate::SignedIn;
use crate::auth::finish;
use crate::email::refuse_unusable;
use crate::events::record;
use crate::mfa::Proof;
use crate::secrets;

/// What a provider said about the person who signed in.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ProviderProfile {
	/// The provider's stable ID for them.
	pub subject: String,
	/// The address it gave.
	pub email: Option<String>,
	/// Whether it vouched for that address.
	pub email_verified: bool,
	/// Their name there.
	pub name: Option<String>,
	/// Their username there.
	pub preferred_username: Option<String>,
	/// A picture of them.
	pub picture: Option<String>,
	/// The provider's tokens, when the app keeps them (`store_tokens`).
	#[serde(default)]
	pub grant: Option<ProviderGrant>,
}

/// The tokens a provider handed over at sign-in, for a provider whose
/// tokens the app keeps. Its `Debug` leaves the tokens out.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ProviderGrant {
	pub access_token: String,
	pub refresh_token: Option<String>,
	/// Seconds the access token works for, if the provider said.
	pub expires_in: Option<i64>,
	/// The scopes granted, as the provider listed them.
	pub scope: Option<String>,
}

impl fmt::Debug for ProviderGrant {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("ProviderGrant")
			.field("expires_in", &self.expires_in)
			.field("scope", &self.scope)
			.finish_non_exhaustive()
	}
}

impl ProviderProfile {
	/// The address, when the provider vouched for it and it's valid.
	fn verified_email(&self) -> Option<Email> {
		self.email
			.as_deref()
			.filter(|_| self.email_verified)
			.and_then(|address| Email::parse(address).ok())
	}

	fn profile(&self) -> Profile {
		let mut profile = Profile {
			email: self.verified_email(),
			name: self.name.clone(),
			preferred_username: self.preferred_username.clone(),
			..Profile::default()
		};
		profile.picture = self
			.picture
			.as_deref()
			.and_then(|picture| leptoskit_db::Url::parse(picture).ok());
		profile
	}
}

/// What a third-party flow is for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Intent {
	/// Signing in, or up.
	SignIn,
	/// Connecting the provider to the signed-in account.
	Link {
		account: Id<Account>,
		session: Id<Session>,
	},
}

/// A third-party flow in progress, sealed in its challenge.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Flow {
	pub provider: String,
	pub verifier: String,
	pub nonce: String,
	pub redirect_to: String,
	pub intent: Intent,
	/// How a native app goes through it; `None` for a browser. Flows sealed
	/// before native apps could sign in have none.
	#[serde(default)]
	pub native: Option<crate::NativeStart>,
}

impl Flow {
	pub(crate) fn seal(&self) -> Result<Vec<u8>> {
		serde_json::to_vec(self).map_err(|_| AuthError::Config("a flow couldn't be stored".into()))
	}

	pub(crate) fn open(bytes: &[u8]) -> Result<Self> {
		serde_json::from_slice(bytes).map_err(|_| AuthError::SignInExpired)
	}
}

/// A sign-up waiting for the person to choose a separate account, because
/// the provider's address belongs to another.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct PendingSignUp {
	provider: String,
	profile: ProviderProfile,
}

/// An offer to merge `absorbed` into `survivor`, made when connecting a
/// provider found it belonged to `absorbed`.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct PendingMerge {
	survivor: Id<Account>,
	absorbed: Id<Account>,
	provider: String,
}

/// What the provider sent back to `/auth/callback/{provider}`.
#[derive(Clone, Default, Deserialize)]
pub struct CallbackParams {
	/// The authorization code.
	#[serde(default)]
	pub code: Option<String>,
	/// The `state` the flow started with.
	#[serde(default)]
	pub state: Option<String>,
	/// Why the provider refused, such as `access_denied`.
	#[serde(default)]
	pub error: Option<String>,
	/// Sign in with Apple's name and address, sent as JSON on the first
	/// sign-in only.
	#[serde(default)]
	pub user: Option<String>,
}

impl fmt::Debug for CallbackParams {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("CallbackParams")
			.field("error", &self.error)
			.finish_non_exhaustive()
	}
}

/// How a third-party sign-in ended.
#[derive(Debug)]
pub enum ProviderResult {
	/// Signed in, or up.
	SignedIn(Box<SignIn>),
	/// The account has a second factor, which the person must use next.
	SecondFactor(crate::SecondFactorChallenge),
	/// Something to show the person, with no new session: connected,
	/// already connected, a merge offer, or an address another account
	/// has.
	Outcome(ProviderOutcome),
}

/// The subject a browser's third-party flows are filed under: a hash of
/// its binding cookie, so only that browser finds them.
pub(crate) fn binding_subject(binding: &SessionToken) -> String {
	use base64::Engine;
	let hash = secrets::digest(&[b"provider-binding", binding.expose().as_bytes()]);
	base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hash)
}

/// Where to go after signing in: a path on this site, never another
/// site. `None` is `/`.
///
/// # Errors
///
/// [`AuthError::InvalidRedirect`] for anything but a path.
pub(crate) fn return_path(path: Option<&str>) -> Result<String> {
	let path = path.unwrap_or("/");
	let valid = path.starts_with('/')
		&& !path.starts_with("//")
		&& !path.starts_with("/\\")
		&& path.len() <= 1024
		&& !path
			.chars()
			.any(|character| character.is_control() || character == '\\');

	if valid {
		Ok(path.to_owned())
	} else {
		Err(AuthError::InvalidRedirect)
	}
}

/// The subject of the merge offer for `session`.
fn merge_subject(session: Id<Session>) -> String {
	format!("merge:{session}")
}

/// The subject of the passkey ceremony proving the other account in a
/// merge.
fn merge_passkey_subject(session: Id<Session>) -> String {
	format!("merge-passkey:{session}")
}

/// How long a merge offer lasts.
const MERGE_LIFETIME: chrono::TimeDelta = chrono::TimeDelta::minutes(10);

impl Auth {
	/// Signs in, signs up, or connects the provider, for a flow whose
	/// provider just said who the person is.
	pub(crate) async fn finish_flow(
		&self,
		flow: &Flow,
		profile: &ProviderProfile,
		binding: &SessionToken,
		request: &RequestInfo,
	) -> Result<ProviderResult> {
		let db = &self.inner.db;
		let existing = Identity::query()
			.filter(|identity| {
				identity.provider.eq(flow.provider.clone())
					& identity.subject.eq(profile.subject.clone())
			})
			.first(db)
			.await?;

		match &flow.intent {
			Intent::SignIn => {
				match existing {
					Some(identity) => self.sign_in_identity(&identity, profile, request).await,
					None => {
						self.sign_up_identity(&flow.provider, profile, binding, request)
							.await
					}
				}
			}
			Intent::Link { account, session } => {
				let live = Session::query()
					.filter(|row| {
						row.id.eq(*session) & row.account_id.eq(*account) & row.revoked_at.is_null()
					})
					.count(db)
					.await?;

				if live == 0 {
					return Err(AuthError::SignedOut);
				}

				match existing {
					Some(identity) if identity.account_id == *account => {
						Ok(ProviderResult::Outcome(ProviderOutcome::AlreadyLinked))
					}
					Some(identity) => {
						self.store_merge_offer(
							*account,
							*session,
							identity.account_id,
							&flow.provider,
						)
						.await?;
						Ok(ProviderResult::Outcome(ProviderOutcome::Merge))
					}
					None => {
						self.link_identity(*account, &flow.provider, profile, request)
							.await?;
						Ok(ProviderResult::Outcome(ProviderOutcome::Linked))
					}
				}
			}
		}
	}

	/// Offers to merge `absorbed` into the signed-in account, after the
	/// person proved they can sign in to it through `via`: a provider's ID,
	/// or `solana` for a wallet. The offer is bound to the session, lasts ten
	/// minutes, and replaces any earlier one.
	pub(crate) async fn offer_merge(
		&self,
		me: &SignedIn,
		absorbed: Id<Account>,
		via: &str,
	) -> Result<()> {
		self.store_merge_offer(me.account, me.session, absorbed, via)
			.await
	}

	async fn store_merge_offer(
		&self,
		survivor: Id<Account>,
		session: Id<Session>,
		absorbed: Id<Account>,
		via: &str,
	) -> Result<()> {
		let offer = PendingMerge {
			survivor,
			absorbed,
			provider: via.to_owned(),
		};
		let state = serde_json::to_vec(&offer)
			.map_err(|_| AuthError::Config("an offer couldn't be stored".into()))?;
		self.store_ceremony_for(
			&self.inner.db,
			Purpose::Merge,
			&merge_subject(session),
			&state,
			MERGE_LIFETIME,
		)
		.await?;
		Ok(())
	}

	/// Signs in with a known identity: a session, or the second step when
	/// the account has a second factor.
	async fn sign_in_identity(
		&self,
		identity: &Identity,
		profile: &ProviderProfile,
		request: &RequestInfo,
	) -> Result<ProviderResult> {
		let tx = self.inner.db.begin().await?;
		let result = async {
			let account = Account::get(&tx, identity.account_id).await?;
			refuse_unusable(&account)?;
			refresh_identity(&tx, identity.id, profile).await?;
			self.keep_grant(&tx, identity, profile).await?;
			let factors = self.factors(&tx, account.id).await?;

			if !factors.is_empty() {
				let challenge = self
					.second_factor_challenge(&tx, account.id, factors, None)
					.await?;
				return Ok(ProviderResult::SecondFactor(challenge));
			}

			let now = leptoskit_db::now();
			let sign_in = self
				.start_session(&tx, account.id, Aal::One, now, None, request)
				.await?;
			record(&tx, Some(account.id), EventKind::SignedIn, request).await?;
			Ok(ProviderResult::SignedIn(Box::new(sign_in)))
		}
		.await;
		finish(tx, result).await
	}

	/// Creates an account for a new identity, unless the address the
	/// provider vouched for is already another account's: then the person
	/// chooses, and nothing is created yet.
	async fn sign_up_identity(
		&self,
		provider: &str,
		profile: &ProviderProfile,
		binding: &SessionToken,
		request: &RequestInfo,
	) -> Result<ProviderResult> {
		let db = &self.inner.db;

		if let Some(address) = profile.verified_email() {
			let taken = EmailAddress::query()
				.filter(|email| email.address.eq(&address))
				.count(db)
				.await?;

			if taken > 0 {
				let pending = PendingSignUp {
					provider: provider.to_owned(),
					profile: profile.clone(),
				};
				let state = serde_json::to_vec(&pending)
					.map_err(|_| AuthError::Config("a sign-up couldn't be stored".into()))?;
				self.store_ceremony_for(
					db,
					Purpose::ProviderSignUp,
					&binding_subject(binding),
					&state,
					MERGE_LIFETIME,
				)
				.await?;
				return Ok(ProviderResult::Outcome(ProviderOutcome::EmailInUse));
			}
		}

		let tx = db.begin().await?;
		let result = self
			.create_identity_account(&tx, provider, profile, request)
			.await;
		finish(tx, result)
			.await
			.map(|sign_in| ProviderResult::SignedIn(Box::new(sign_in)))
	}

	/// After [`ProviderOutcome::EmailInUse`]: creates the separate account
	/// the person chose, without the address another account has.
	///
	/// # Errors
	///
	/// [`AuthError::SignInExpired`] when there's no sign-up waiting for this
	/// browser.
	pub async fn continue_provider_sign_up(
		&self,
		binding: &SessionToken,
		request: &RequestInfo,
	) -> Result<SignIn> {
		let state = self
			.take_ceremony(Purpose::ProviderSignUp, &binding_subject(binding), None)
			.await?
			.ok_or(AuthError::SignInExpired)?;
		let pending: PendingSignUp =
			serde_json::from_slice(&state).map_err(|_| AuthError::SignInExpired)?;
		let profile = ProviderProfile {
			email_verified: false,
			..pending.profile
		};
		let tx = self.inner.db.begin().await?;
		let result = self
			.create_identity_account(&tx, &pending.provider, &profile, request)
			.await;
		finish(tx, result).await
	}

	async fn create_identity_account(
		&self,
		tx: &Tx,
		provider: &str,
		profile: &ProviderProfile,
		request: &RequestInfo,
	) -> Result<SignIn> {
		let account = NewAccount {
			disabled_at: None,
			locked_until: None,
		}
		.insert(tx)
		.await?;
		let identity = insert_identity(tx, account.id, provider, profile).await?;
		self.keep_grant(tx, &identity, profile).await?;

		if let Some(address) = profile.verified_email() {
			NewEmailAddress {
				account_id: account.id,
				address,
				verified_at: Some(leptoskit_db::now()),
				is_primary: true,
			}
			.insert(tx)
			.await?;
		}

		(self.inner.sign_up)(tx, account.id, &profile.profile()).await?;
		record(tx, Some(account.id), EventKind::SignedUp, request).await?;
		let now = leptoskit_db::now();
		let mut sign_in = self
			.start_session(tx, account.id, Aal::One, now, None, request)
			.await?;
		record(tx, Some(account.id), EventKind::SignedIn, request).await?;
		sign_in.signed_up = true;
		Ok(sign_in)
	}

	/// Attaches a new identity to `account`, with its address when the
	/// provider vouched for it and no one has it yet.
	async fn link_identity(
		&self,
		account: Id<Account>,
		provider: &str,
		profile: &ProviderProfile,
		request: &RequestInfo,
	) -> Result<()> {
		let tx = self.inner.db.begin().await?;
		let result = async {
			let identity = insert_identity(&tx, account, provider, profile).await?;
			self.keep_grant(&tx, &identity, profile).await?;

			if let Some(address) = profile.verified_email() {
				let taken = EmailAddress::query()
					.filter(|email| email.address.eq(&address))
					.count(&tx)
					.await?;

				if taken == 0 {
					NewEmailAddress {
						account_id: account,
						address,
						verified_at: Some(leptoskit_db::now()),
						is_primary: false,
					}
					.insert(&tx)
					.await?;
				}
			}

			record(&tx, Some(account), EventKind::IdentityLinked, request).await
		}
		.await;
		finish(tx, result).await?;
		let name = self.provider_display_name(provider);
		self.notify(
			account,
			&format!("{name} was connected"),
			&format!(
				"Your {name} account can now sign in to your account. If you didn't connect it, \
				 sign in, disconnect it, and sign out everywhere."
			),
		)
		.await;
		Ok(())
	}

	/// The signed-in person's connected accounts, newest first.
	///
	/// # Errors
	///
	/// When the database fails.
	pub async fn identities(&self, account: Id<Account>) -> Result<Vec<IdentityInfo>> {
		let identities = Identity::query()
			.filter(|identity| identity.account_id.eq(account))
			.order_by(|identity| identity.created_at.desc())
			.all(&self.inner.db)
			.await?;
		Ok(identities
			.into_iter()
			.map(|identity| {
				IdentityInfo {
					id: identity.id,
					provider_name: self.provider_display_name(&identity.provider),
					provider: identity.provider,
					name: identity.name,
					email: identity.email,
					created_at: identity.created_at,
					last_used_at: identity.last_used_at,
				}
			})
			.collect())
	}

	/// Disconnects one of the signed-in person's connected accounts, after a
	/// fresh sign-in or step-up. The account must keep a way to sign in.
	///
	/// # Errors
	///
	/// [`AuthError::NotFound`], [`AuthError::LastSignInMethod`], or as
	/// [`Auth::begin_totp`] when the session isn't fresh.
	pub async fn unlink_identity(
		&self,
		me: &SignedIn,
		id: Id<Identity>,
		request: &RequestInfo,
	) -> Result<()> {
		self.require_fresh(&self.inner.db, me).await?;
		let tx = self.inner.db.begin().await?;
		let result = async {
			let identity = Identity::query()
				.filter(|row| row.id.eq(id) & row.account_id.eq(me.account))
				.first(&tx)
				.await?
				.ok_or(AuthError::NotFound)?;
			let ended = self
				.take_provider_tokens(&tx, me.account, Some(identity.id))
				.await?;
			Identity::query()
				.filter(|row| row.id.eq(identity.id))
				.delete(&tx)
				.await?;

			if self.sign_in_methods(&tx, me.account).await?.is_empty() {
				return Err(AuthError::LastSignInMethod);
			}

			record(&tx, Some(me.account), EventKind::IdentityUnlinked, request).await?;
			Ok((identity.provider, ended))
		}
		.await;
		let (provider, ended) = finish(tx, result).await?;
		self.revoke_provider_tokens(ended).await;
		let name = self.provider_display_name(&provider);
		self.notify(
			me.account,
			&format!("{name} was disconnected"),
			&format!(
				"Your {name} account can no longer sign in to your account. If you didn't do \
				 this, sign in and sign out everywhere."
			),
		)
		.await;
		Ok(())
	}

	/// What merging would do, when connecting a provider found another
	/// account: `None` when there's no offer for this session.
	///
	/// # Errors
	///
	/// When the database fails.
	pub async fn merge_offer(&self, me: &SignedIn) -> Result<Option<MergeOffer>> {
		let Some((offer, expires_at)) = self.peek_merge(me).await? else {
			return Ok(None);
		};
		let db = &self.inner.db;
		let Some(absorbed) = Account::find(db, offer.absorbed).await? else {
			return Ok(None);
		};
		let emails = EmailAddress::query()
			.filter(|email| email.account_id.eq(absorbed.id) & email.verified_at.is_not_null())
			.all(db)
			.await?;
		let identities = Identity::query()
			.filter(|identity| identity.account_id.eq(absorbed.id))
			.all(db)
			.await?;
		let passkeys = Passkey::query()
			.filter(|passkey| passkey.account_id.eq(absorbed.id))
			.count(db)
			.await?;
		let wallets = crate::Wallet::query()
			.filter(|wallet| wallet.account_id.eq(absorbed.id))
			.all(db)
			.await?;
		let data = self.account_data()?;
		let mut modes = vec![MergeMode::MoveLogin];

		if self.can_bring_data(&data) {
			modes.insert(0, MergeMode::BringData);
		}

		let expires_in = (expires_at - leptoskit_db::now()).num_seconds().max(0);
		Ok(Some(MergeOffer {
			provider: offer.provider,
			created_at: absorbed.created_at,
			emails: emails
				.into_iter()
				.map(|email| email.address.to_string())
				.collect(),
			identities: identities
				.into_iter()
				.map(|identity| self.provider_display_name(&identity.provider))
				.collect(),
			passkeys: u32::try_from(passkeys).unwrap_or(u32::MAX),
			wallets: wallets.into_iter().map(|wallet| wallet.address).collect(),
			data: self.merge_data(&data, absorbed.id).await?,
			second_factor: self.factors(db, absorbed.id).await?,
			modes,
			expires_in: u64::try_from(expires_in).unwrap_or_default(),
		}))
	}

	/// Asks the other account's passkeys to prove the person controls it,
	/// for a merge: the options for `navigator.credentials.get`.
	///
	/// # Errors
	///
	/// [`AuthError::SignInExpired`] without an offer, and
	/// [`AuthError::NotEnrolled`] when the other account has no passkeys.
	pub async fn begin_merge_passkey(
		&self,
		me: &SignedIn,
	) -> Result<crate::RequestChallengeResponse> {
		let (offer, _) = self.peek_merge(me).await?.ok_or(AuthError::SignInExpired)?;
		self.passkey_assertion(
			&self.inner.db,
			offer.absorbed,
			&merge_passkey_subject(me.session),
		)
		.await
	}

	/// Drops the merge offer, if there is one.
	///
	/// # Errors
	///
	/// When the database fails.
	pub async fn cancel_merge(&self, me: &SignedIn) -> Result<()> {
		self.take_ceremony(Purpose::Merge, &merge_subject(me.session), None)
			.await?;
		Ok(())
	}

	/// Merges the other account into the signed-in one, as the offer
	/// described, once the person confirms. Both modes move the other
	/// account's emails, connected accounts, passkeys, and wallets here,
	/// discard its second factors, end its sessions, and delete it. "Move
	/// only the login" deletes its data with it; "bring its data" first
	/// moves its rows in the app's tables here, each table by its
	/// `on_account_merge` strategy, in the same transaction. A tombstone
	/// keeps its ID resolving here. It can't be undone.
	///
	/// It needs a fresh sign-in or step-up on this account, and the other
	/// account's own second factor when it has one, so a merge can't get
	/// around either.
	///
	/// # Errors
	///
	/// [`AuthError::SignInExpired`] without an offer;
	/// [`AuthError::Reauthenticate`] or [`AuthError::StepUpRequired`];
	/// [`AuthError::InvalidCode`] or [`AuthError::PasskeyFailed`] for the
	/// other account's factor; [`AuthError::MergeConflict`] when its data
	/// can't move, which changes nothing; or [`AuthError::Unsupported`]
	/// without `confirm`, or for "bring its data" when a table doesn't say
	/// how its rows move.
	pub async fn merge_accounts(
		&self,
		me: &SignedIn,
		merge: &MergeAccounts,
		request: &RequestInfo,
	) -> Result<()> {
		if !merge.confirm {
			return Err(AuthError::Unsupported(
				"confirm the merge: it can't be undone",
			));
		}

		let data = match merge.mode {
			MergeMode::MoveLogin => None,
			MergeMode::BringData => {
				let data = self.account_data()?;

				if !self.can_bring_data(&data) {
					return Err(AuthError::Unsupported(
						"this app can't bring the other account's data along yet: every table \
						 that holds it must say how with `on_account_merge`",
					));
				}

				Some(data)
			}
		};
		let db = &self.inner.db;
		self.require_fresh(db, me).await?;
		let (offer, _) = self.peek_merge(me).await?.ok_or(AuthError::SignInExpired)?;

		if offer.survivor != me.account {
			return Err(AuthError::SignInExpired);
		}

		let factors = self.factors(db, offer.absorbed).await?;
		let subject = merge_passkey_subject(me.session);
		let proof = match (&merge.code, &merge.credential) {
			(_, Some(credential)) => {
				Some(Proof::Passkey {
					credential,
					subject: &subject,
				})
			}
			(Some(code), None) => Some(Proof::Code(code)),
			(None, None) => None,
		};

		if !factors.is_empty() {
			let Some(proof) = proof else {
				return Err(AuthError::StepUpRequired);
			};
			self.limit(
				&[b"second-factor-account", offer.absorbed.uuid().as_bytes()],
				self.inner.limits.second_factor_per_account,
			)
			.await?;

			if !self.prove(offer.absorbed, proof, request).await? {
				self.second_factor_failed(offer.absorbed, request).await?;
				return Err(match proof {
					Proof::Code(_) => AuthError::InvalidCode,
					Proof::Passkey { .. } => AuthError::PasskeyFailed,
				});
			}
		}

		// The offer works once.
		self.take_ceremony(Purpose::Merge, &merge_subject(me.session), None)
			.await?
			.ok_or(AuthError::SignInExpired)?;
		let tx = db.begin().await?;
		let result = async {
			if let Some(data) = &data {
				self.bring_data(&tx, data, me.account, offer.absorbed)
					.await?;
			}

			self.move_login(&tx, me.account, offer.absorbed, request)
				.await
		}
		.await;
		let ended = finish(tx, result).await?;
		self.revoke_provider_tokens(ended).await;
		let text = if data.is_some() {
			"Another account's sign-in methods and data were moved into this one, and the other \
			 account was deleted. If you didn't do this, sign in and sign out everywhere, then \
			 contact support."
		} else {
			"Another account's sign-in methods were moved into this one, and the other account and \
			 its data were deleted. If you didn't do this, sign in and sign out everywhere, then \
			 contact support."
		};
		self.notify(me.account, "Two accounts were merged", text)
			.await;
		Ok(())
	}

	/// Whether `proof` is a working second factor of `account`'s, used up
	/// when it is.
	async fn prove(
		&self,
		account: Id<Account>,
		proof: Proof<'_>,
		_request: &RequestInfo,
	) -> Result<bool> {
		let ceremony = match proof {
			Proof::Code(_) => None,
			Proof::Passkey { subject, .. } => {
				self.take_ceremony(Purpose::PasskeyAuthentication, subject, None)
					.await?
			}
		};
		let tx = self.inner.db.begin().await?;
		let result = self.use_proof(&tx, account, proof, ceremony).await;
		finish(tx, result).await
	}

	/// The merge itself, in its transaction. Returns the provider tokens
	/// the absorbed account kept, to revoke once it commits: they were
	/// granted to that account, and its identities sign in again here to
	/// keep new ones.
	async fn move_login(
		&self,
		tx: &Tx,
		survivor: Id<Account>,
		absorbed: Id<Account>,
		request: &RequestInfo,
	) -> Result<Vec<crate::provider_tokens::Ended>> {
		let ended = self.take_provider_tokens(tx, absorbed, None).await?;
		Identity::update()
			.filter(|row| row.account_id.eq(absorbed))
			.set(|row| row.account_id, survivor)
			.run(tx)
			.await?;
		Passkey::update()
			.filter(|row| row.account_id.eq(absorbed))
			.set(|row| row.account_id, survivor)
			.run(tx)
			.await?;
		// Wallets keep whether they may sign in: the person proved they could
		// sign in to the other account, which is where that was chosen.
		crate::Wallet::update()
			.filter(|row| row.account_id.eq(absorbed))
			.set(|row| row.account_id, survivor)
			.run(tx)
			.await?;
		EmailAddress::query()
			.filter(|row| row.account_id.eq(absorbed) & row.verified_at.is_null())
			.delete(tx)
			.await?;
		EmailAddress::update()
			.filter(|row| row.account_id.eq(absorbed))
			.set(|row| row.account_id, survivor)
			.set(|row| row.is_primary, false)
			.run(tx)
			.await?;
		// The other account's history stays, on the account it became.
		Event::update()
			.filter(|row| row.account_id.eq(Some(absorbed)))
			.set(|row| row.account_id, Some(survivor))
			.run(tx)
			.await?;
		// Older merges into the absorbed account now point here.
		MergedAccount::update()
			.filter(|row| row.survivor_id.eq(absorbed))
			.set(|row| row.survivor_id, survivor)
			.run(tx)
			.await?;
		// Deleting the account ends its sessions and discards its second
		// factors, and the app's own rows go with it through their
		// `on_delete = cascade`.
		Account::query()
			.filter(|row| row.id.eq(absorbed))
			.delete(tx)
			.await?;
		NewMergedAccount {
			absorbed_id: absorbed,
			survivor_id: survivor,
		}
		.insert(tx)
		.await?;
		record(tx, Some(survivor), EventKind::AccountMerged, request).await?;
		Ok(ended)
	}

	/// The account `account` is now: itself, or the one it was merged into.
	///
	/// # Errors
	///
	/// When the database fails.
	pub async fn resolve_merged(&self, account: Id<Account>) -> Result<Id<Account>> {
		let tombstone = MergedAccount::query()
			.filter(|row| row.absorbed_id.eq(account))
			.first(&self.inner.db)
			.await?;
		Ok(tombstone.map_or(account, |row| row.survivor_id))
	}

	/// The merge offer for `me`'s session, without using it up.
	async fn peek_merge(
		&self,
		me: &SignedIn,
	) -> Result<Option<(PendingMerge, leptoskit_db::Timestamp)>> {
		let subject = merge_subject(me.session);
		let Some((state, expires_at)) = self.peek_ceremony(Purpose::Merge, &subject).await? else {
			return Ok(None);
		};
		let offer: PendingMerge =
			serde_json::from_slice(&state).map_err(|_| AuthError::SignInExpired)?;
		Ok((offer.survivor == me.account).then_some((offer, expires_at)))
	}

	/// What a provider is called, for people.
	#[cfg_attr(
		not(feature = "oauth"),
		allow(clippy::unused_self, reason = "the names come from the providers")
	)]
	pub(crate) fn provider_display_name(&self, id: &str) -> String {
		#[cfg(feature = "oauth")]
		{
			self.provider_name(id)
		}
		#[cfg(not(feature = "oauth"))]
		{
			id.to_owned()
		}
	}

	/// The providers people can sign in with: none without the `oauth`
	/// feature.
	#[cfg(not(feature = "oauth"))]
	pub fn providers(&self) -> Vec<crate::ProviderInfo> {
		Vec::new()
	}
}

/// Records a new identity.
async fn insert_identity(
	connection: &impl Connection,
	account: Id<Account>,
	provider: &str,
	profile: &ProviderProfile,
) -> Result<Identity> {
	let identity = NewIdentity {
		account_id: account,
		provider: provider.to_owned(),
		subject: profile.subject.clone(),
		email: profile.email.as_deref().map(truncate::<320>),
		email_verified: profile.email_verified,
		name: profile
			.name
			.as_deref()
			.or(profile.preferred_username.as_deref())
			.map(truncate::<200>),
		last_used_at: Some(leptoskit_db::now()),
	}
	.insert(connection)
	.await?;
	Ok(identity)
}

/// Keeps what the provider says now, and when it was used.
async fn refresh_identity(
	connection: &impl Connection,
	identity: Id<Identity>,
	profile: &ProviderProfile,
) -> Result<()> {
	Identity::update()
		.filter(|row| row.id.eq(identity))
		.set(
			|row| row.email,
			profile.email.as_deref().map(truncate::<320>),
		)
		.set(|row| row.email_verified, profile.email_verified)
		.set(|row| row.last_used_at, Some(leptoskit_db::now()))
		.run(connection)
		.await?;
	Ok(())
}

fn truncate<const N: usize>(text: &str) -> String {
	text.chars().take(N).collect()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn only_paths_on_this_site_are_returned_to() {
		assert_eq!(return_path(None).unwrap(), "/");
		assert_eq!(
			return_path(Some("/settings?tab=1")).unwrap(),
			"/settings?tab=1"
		);

		for bad in [
			"https://evil.example",
			"//evil.example",
			"/\\evil.example",
			"settings",
			"/a\\b",
			"/line\nbreak",
			"",
		] {
			assert!(
				matches!(return_path(Some(bad)), Err(AuthError::InvalidRedirect)),
				"{bad:?}"
			);
		}

		assert!(return_path(Some(&format!("/{}", "a".repeat(1024)))).is_err());
	}

	#[test]
	fn only_vouched_for_addresses_count() {
		let profile = ProviderProfile {
			subject: "1".into(),
			email: Some("Ada@Example.com".into()),
			email_verified: true,
			picture: Some("https://example.com/ada.png".into()),
			..ProviderProfile::default()
		};
		assert_eq!(
			profile.verified_email().map(|email| email.to_string()),
			Some("ada@example.com".into())
		);
		assert!(profile.profile().picture.is_some());
		let unverified = ProviderProfile {
			email_verified: false,
			..profile
		};
		assert_eq!(unverified.verified_email(), None);
		assert_eq!(unverified.profile().email, None);
	}
}
