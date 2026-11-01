//! Solana wallets: linked to an account with proof that its owner holds
//! the key, re-proved when an action needs a fresh signature, and used to
//! sign in only when the owner turns that on.
//!
//! Every proof is a Sign-In With Solana message the server issued: bound to
//! `[auth] origin`'s host (which wallets compare with the page asking), with
//! a single-use nonce that expires after five minutes, kept sealed in
//! `auth.challenges` and claimed before anything is checked. The signed
//! text must be, byte for byte, the message issued for the address that
//! signed, and the signature must verify as strict ed25519.

use std::time::Duration;

use leptoskit_db::Connection;
use leptoskit_db::Id;
use leptoskit_db::Pubkey;
use subtle::ConstantTimeEq;

use crate::Aal;
use crate::Account;
use crate::Auth;
use crate::AuthError;
use crate::EventKind;
use crate::LinkWallet;
use crate::NewWallet;
use crate::PublicWallet;
use crate::Purpose;
use crate::RequestInfo;
use crate::Result;
use crate::SessionToken;
use crate::SignInStep;
use crate::SignedIn;
use crate::SiwsInput;
use crate::SiwsMessage;
use crate::UpdateWallet;
use crate::Wallet;
use crate::WalletInfo;
use crate::WalletLinked;
use crate::WalletProof;
use crate::WalletSettings;
use crate::auth::finish;
use crate::email::refuse_unusable;
use crate::events::record;
use crate::passkeys::CEREMONY_LIFETIME;

/// The chain every wallet here is on.
pub(crate) const CHAIN: &str = "solana";

/// The longest wallet label kept, in characters.
const LABEL_LENGTH: usize = 100;

/// The subject every wallet sign-in challenge is filed under; its token
/// finds it.
const SIGN_IN_SUBJECT: &str = "wallet-sign-in";

/// How wallets work on this server: whether they may sign in, and which
/// cluster messages name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SolanaOptions {
	/// Whether a wallet may sign in, once its owner turns that on.
	pub sign_in: bool,
	/// The cluster, such as `mainnet`.
	pub chain_id: String,
}

impl Default for SolanaOptions {
	fn default() -> Self {
		Self {
			sign_in: false,
			chain_id: "mainnet".to_owned(),
		}
	}
}

/// A wallet sign-in that has started: what the wallet must sign, and the
/// token that matches its answer, which a browser keeps in an `HttpOnly`
/// cookie.
#[derive(Debug)]
pub struct WalletSignInChallenge {
	/// Matches the answer to the challenge.
	pub token: SessionToken,
	/// What the wallet signs.
	pub input: SiwsInput,
}

impl From<Wallet> for WalletInfo {
	fn from(wallet: Wallet) -> Self {
		Self {
			id: wallet.id,
			chain: wallet.chain,
			address: wallet.address,
			label: wallet.label,
			can_sign_in: wallet.can_sign_in,
			is_public: wallet.is_public,
			verified_at: wallet.verified_at,
			last_proved_at: wallet.last_proved_at,
		}
	}
}

impl Auth {
	/// Whether wallets work here, whether they may sign in, and the
	/// cluster messages name.
	pub fn wallet_settings(&self) -> WalletSettings {
		WalletSettings {
			enabled: cfg!(feature = "solana") && self.inner.origin.is_some(),
			sign_in: self.wallet_sign_in_enabled(),
			chain_id: self.inner.solana.chain_id.clone(),
		}
	}

	fn wallet_sign_in_enabled(&self) -> bool {
		cfg!(feature = "solana") && self.inner.origin.is_some() && self.inner.solana.sign_in
	}

	/// Starts linking a wallet to the signed-in account: the message its
	/// wallet must sign. Needs a fresh sign-in or step-up, like any change to
	/// how the account signs in.
	///
	/// # Errors
	///
	/// [`AuthError::Reauthenticate`] or [`AuthError::StepUpRequired`],
	/// [`AuthError::RateLimited`], or [`AuthError::Unsupported`] without the
	/// `solana` feature or an origin.
	pub async fn begin_wallet_link(
		&self,
		me: &SignedIn,
		request: &RequestInfo,
	) -> Result<SiwsInput> {
		let input = self.siws_input(
			&format!(
				"Link this wallet to your account on {}.",
				self.statement_name()
			),
			"link",
			None,
		)?;
		self.require_fresh(&self.inner.db, me).await?;
		self.limit_wallet_start(request).await?;
		self.issue(Purpose::WalletLink, &link_subject(me.session), &input)
			.await?;
		Ok(input)
	}

	/// Links the wallet that signed the message [`Auth::begin_wallet_link`]
	/// issued. A wallet already linked here just gets a fresh proof. A wallet
	/// linked to another account that it may sign in to becomes a merge
	/// offer, since the signature proves the person can sign in there; one
	/// that may not sign in there is refused.
	///
	/// # Errors
	///
	/// [`AuthError::WalletFailed`] when the challenge is unknown, used, or
	/// expired, or the proof doesn't check out; [`AuthError::WalletInUse`].
	pub async fn finish_wallet_link(
		&self,
		me: &SignedIn,
		link: &LinkWallet,
		request: &RequestInfo,
	) -> Result<WalletLinked> {
		let issued = self
			.claim(Purpose::WalletLink, &link_subject(me.session), None)
			.await?;
		self.checked(&issued, &link.proof, Some(me.account), request)
			.await?;
		let db = &self.inner.db;
		let address = &link.proof.address;
		let existing = wallet_at(db, address).await?;

		match existing {
			Some(wallet) if wallet.account_id == me.account => {
				let wallet = self.proved(&wallet, &link.proof, request).await?;
				Ok(WalletLinked::AlreadyLinked(wallet))
			}

			Some(wallet) => {
				if !(wallet.can_sign_in && self.wallet_sign_in_enabled()) {
					return Err(AuthError::WalletInUse);
				}

				self.offer_merge(me, wallet.account_id, CHAIN).await?;
				Ok(WalletLinked::Merge)
			}
			None => {
				let tx = db.begin().await?;
				let result = async {
					let now = leptoskit_db::now();
					let wallet = NewWallet {
						account_id: me.account,
						chain: CHAIN.to_owned(),
						address: address.clone(),
						label: link.label.as_deref().and_then(label),
						can_sign_in: false,
						is_public: false,
						verified_at: now,
						last_proved_at: now,
						proof: link.proof.message.clone(),
						proof_signature: link.proof.signature.clone(),
					}
					.insert(&tx)
					.await
					.map_err(|error| {
						// Someone linked it a moment ago.
						if matches!(error, leptoskit_db::DbError::Conflict { .. }) {
							AuthError::WalletInUse
						} else {
							error.into()
						}
					})?;
					record(&tx, Some(me.account), EventKind::WalletLinked, request).await?;
					Ok(wallet)
				}
				.await;
				let wallet = finish(tx, result).await?;
				self.notify(
					me.account,
					"A wallet was linked",
					&format!(
						"The Solana wallet {address} was linked to your account. It can't sign in \
						 unless you turn that on. If you didn't link it, sign in, unlink it, and \
						 sign out everywhere."
					),
				)
				.await;
				Ok(WalletLinked::Linked(wallet.into()))
			}
		}
	}

	/// The account's linked wallets, newest first.
	///
	/// # Errors
	///
	/// When the database fails.
	pub async fn wallets(&self, account: Id<Account>) -> Result<Vec<WalletInfo>> {
		let wallets = Wallet::query()
			.filter(|wallet| wallet.account_id.eq(account))
			.order_by(|wallet| wallet.created_at.desc())
			.all(&self.inner.db)
			.await?;
		Ok(wallets.into_iter().map(WalletInfo::from).collect())
	}

	/// The wallets the account's owner chose to show, for their public
	/// profile: the address and label only.
	///
	/// # Errors
	///
	/// When the database fails.
	pub async fn public_wallets(&self, account: Id<Account>) -> Result<Vec<PublicWallet>> {
		let wallets = Wallet::query()
			.filter(|wallet| wallet.account_id.eq(account) & wallet.is_public.eq(true))
			.order_by(|wallet| wallet.created_at.asc())
			.all(&self.inner.db)
			.await?;
		Ok(wallets
			.into_iter()
			.map(|wallet| {
				PublicWallet {
					chain: wallet.chain,
					address: wallet.address,
					label: wallet.label,
				}
			})
			.collect())
	}

	/// The addresses of the account's linked wallets, which policies read as
	/// `cx.wallets()`.
	///
	/// # Errors
	///
	/// When the database fails.
	pub async fn wallet_addresses(&self, account: Id<Account>) -> Result<Vec<Pubkey>> {
		let wallets = Wallet::query()
			.filter(|wallet| wallet.account_id.eq(account))
			.all(&self.inner.db)
			.await?;
		Ok(wallets.into_iter().map(|wallet| wallet.address).collect())
	}

	/// Changes a linked wallet's label, whether it's public, or whether it
	/// may sign in. Changing whether it signs in needs a fresh sign-in or
	/// step-up, emails the owner, and must leave the account a way to sign
	/// in.
	///
	/// # Errors
	///
	/// [`AuthError::NotFound`], [`AuthError::Unsupported`] when wallets
	/// can't sign in on this server, [`AuthError::Reauthenticate`] or
	/// [`AuthError::StepUpRequired`], or [`AuthError::LastSignInMethod`].
	pub async fn update_wallet(
		&self,
		me: &SignedIn,
		update: &UpdateWallet,
		request: &RequestInfo,
	) -> Result<WalletInfo> {
		let db = &self.inner.db;
		let wallet = owned(db, me.account, update.id).await?;
		let sign_in = update
			.can_sign_in
			.filter(|sign_in| *sign_in != wallet.can_sign_in);

		if let Some(sign_in) = sign_in {
			if sign_in && !self.wallet_sign_in_enabled() {
				return Err(AuthError::Unsupported(
					"signing in with wallets is off on this server",
				));
			}

			self.require_fresh(db, me).await?;
		}

		let tx = db.begin().await?;
		let result = async {
			if let Some(text) = &update.label {
				Wallet::update()
					.filter(|row| row.id.eq(wallet.id))
					.set(|row| row.label, label(text))
					.run(&tx)
					.await?;
			}

			if let Some(public) = update.is_public {
				Wallet::update()
					.filter(|row| row.id.eq(wallet.id))
					.set(|row| row.is_public, public)
					.run(&tx)
					.await?;
			}

			if let Some(sign_in) = sign_in {
				Wallet::update()
					.filter(|row| row.id.eq(wallet.id))
					.set(|row| row.can_sign_in, sign_in)
					.run(&tx)
					.await?;
			}

			if sign_in.is_some() {
				if self.sign_in_methods(&tx, me.account).await?.is_empty() {
					return Err(AuthError::LastSignInMethod);
				}

				record(
					&tx,
					Some(me.account),
					EventKind::WalletSignInChanged,
					request,
				)
				.await?;
			}

			Ok(Wallet::get(&tx, wallet.id).await?)
		}
		.await;
		let updated = finish(tx, result).await?;

		match sign_in {
			Some(true) => {
				self.notify(
					me.account,
					"A wallet can sign in",
					&format!(
						"The Solana wallet {} can now sign in to your account. If you didn't do \
						 this, sign in, turn it off, and sign out everywhere.",
						updated.address
					),
				)
				.await;
			}
			Some(false) => {
				self.notify(
					me.account,
					"A wallet can no longer sign in",
					&format!(
						"The Solana wallet {} can no longer sign in to your account.",
						updated.address
					),
				)
				.await;
			}
			None => {}
		}

		Ok(updated.into())
	}

	/// Unlinks one of the signed-in person's wallets, after a fresh sign-in
	/// or step-up. The account must keep a way to sign in.
	///
	/// # Errors
	///
	/// [`AuthError::NotFound`], [`AuthError::LastSignInMethod`], or
	/// [`AuthError::Reauthenticate`] or [`AuthError::StepUpRequired`].
	pub async fn unlink_wallet(
		&self,
		me: &SignedIn,
		id: Id<Wallet>,
		request: &RequestInfo,
	) -> Result<()> {
		let db = &self.inner.db;
		self.require_fresh(db, me).await?;
		let tx = db.begin().await?;
		let result = async {
			let wallet = owned(&tx, me.account, id).await?;
			Wallet::query()
				.filter(|row| row.id.eq(wallet.id))
				.delete(&tx)
				.await?;

			if self.sign_in_methods(&tx, me.account).await?.is_empty() {
				return Err(AuthError::LastSignInMethod);
			}

			record(&tx, Some(me.account), EventKind::WalletUnlinked, request).await?;
			Ok(wallet.address)
		}
		.await;
		let address = finish(tx, result).await?;
		self.notify(
			me.account,
			"A wallet was unlinked",
			&format!(
				"The Solana wallet {address} is no longer linked to your account. If you didn't \
				 do this, sign in and sign out everywhere."
			),
		)
		.await;
		Ok(())
	}

	/// Asks one of the signed-in person's wallets to prove it's still
	/// theirs: the message it must sign, naming its address.
	///
	/// # Errors
	///
	/// [`AuthError::NotFound`] when it isn't theirs,
	/// [`AuthError::RateLimited`], or [`AuthError::Unsupported`].
	pub async fn begin_wallet_proof(
		&self,
		me: &SignedIn,
		address: &Pubkey,
		request: &RequestInfo,
	) -> Result<SiwsInput> {
		let input = self.siws_input(
			&format!(
				"Confirm this wallet is still yours on {}.",
				self.statement_name()
			),
			"proof",
			Some(address),
		)?;
		let theirs = wallet_at(&self.inner.db, address)
			.await?
			.is_some_and(|wallet| wallet.account_id == me.account);

		if !theirs {
			return Err(AuthError::NotFound);
		}

		self.limit_wallet_start(request).await?;
		self.issue(Purpose::WalletProof, &proof_subject(me.session), &input)
			.await?;
		Ok(input)
	}

	/// Records a fresh proof from one of the signed-in person's wallets, for
	/// the message [`Auth::begin_wallet_proof`] issued.
	///
	/// # Errors
	///
	/// [`AuthError::WalletFailed`] when it doesn't check out, or
	/// [`AuthError::NotFound`] when the wallet was unlinked meanwhile.
	pub async fn finish_wallet_proof(
		&self,
		me: &SignedIn,
		proof: &WalletProof,
		request: &RequestInfo,
	) -> Result<WalletInfo> {
		let issued = self
			.claim(Purpose::WalletProof, &proof_subject(me.session), None)
			.await?;
		self.checked(&issued, proof, Some(me.account), request)
			.await?;
		let wallet = wallet_at(&self.inner.db, &proof.address)
			.await?
			.filter(|wallet| wallet.account_id == me.account)
			.ok_or(AuthError::NotFound)?;
		self.proved(&wallet, proof, request).await
	}

	/// The signed-in person's wallet at `address`, if it signed a proof
	/// within `within`: for payouts and other actions that should know the
	/// person holds the key right now, not only when they linked it.
	///
	/// ```ignore
	/// let wallet = auth.require_wallet_proof(&me, &payout.address, Duration::from_secs(120)).await?;
	/// ```
	///
	/// # Errors
	///
	/// [`AuthError::ProofRequired`] when its last proof is older, which the
	/// client answers with `prove_wallet`; [`AuthError::NotFound`] when it
	/// isn't theirs.
	pub async fn require_wallet_proof(
		&self,
		me: &SignedIn,
		address: &Pubkey,
		within: Duration,
	) -> Result<Wallet> {
		let wallet = wallet_at(&self.inner.db, address)
			.await?
			.filter(|wallet| wallet.account_id == me.account)
			.ok_or(AuthError::NotFound)?;
		let window = chrono::TimeDelta::from_std(within).unwrap_or(chrono::TimeDelta::MAX);

		if leptoskit_db::now() - wallet.last_proved_at > window {
			return Err(AuthError::ProofRequired);
		}

		Ok(wallet)
	}

	/// Starts signing in with a wallet: the message to sign, and the token
	/// that matches the answer to it.
	///
	/// # Errors
	///
	/// [`AuthError::Unsupported`] when wallets can't sign in here, or
	/// [`AuthError::RateLimited`].
	pub async fn begin_wallet_sign_in(
		&self,
		request: &RequestInfo,
	) -> Result<WalletSignInChallenge> {
		if !self.wallet_sign_in_enabled() {
			return Err(sign_in_off());
		}

		let input = self.siws_input(
			&format!("Sign in to {}.", self.statement_name()),
			"sign-in",
			None,
		)?;
		self.limit_wallet_start(request).await?;
		let state = serde_json::to_vec(&input)
			.map_err(|_| AuthError::Config("a challenge couldn't be stored".into()))?;
		let token = self
			.store_ceremony(
				&self.inner.db,
				Purpose::WalletSignIn,
				SIGN_IN_SUBJECT,
				&state,
			)
			.await?;
		Ok(WalletSignInChallenge { token, input })
	}

	/// Signs in with a wallet that signed the message
	/// [`Auth::begin_wallet_sign_in`] issued, when its owner let it sign
	/// in. A wallet is one factor: an account with a second factor gets the
	/// second step.
	///
	/// # Errors
	///
	/// [`AuthError::WalletFailed`], [`AuthError::UnknownWallet`] when no
	/// account lets this wallet sign in, [`AuthError::Disabled`] or
	/// [`AuthError::Locked`], [`AuthError::RateLimited`], or
	/// [`AuthError::Unsupported`].
	pub async fn finish_wallet_sign_in(
		&self,
		token: &SessionToken,
		proof: &WalletProof,
		request: &RequestInfo,
	) -> Result<SignInStep> {
		if !self.wallet_sign_in_enabled() {
			return Err(sign_in_off());
		}

		if let Some(ip) = request.ip_text() {
			self.limit(
				&[b"wallet-ip", ip.as_bytes()],
				self.inner.limits.verify_per_ip,
			)
			.await?;
		}

		let issued = self
			.claim(Purpose::WalletSignIn, SIGN_IN_SUBJECT, Some(token))
			.await?;
		self.checked(&issued, proof, None, request).await?;
		let db = &self.inner.db;
		let wallet = wallet_at(db, &proof.address)
			.await?
			.filter(|wallet| wallet.can_sign_in)
			.ok_or(AuthError::UnknownWallet)?;
		let tx = db.begin().await?;
		let result = async {
			let account = Account::get(&tx, wallet.account_id).await?;
			refuse_unusable(&account)?;
			set_proof(&tx, &wallet, proof).await?;
			let factors = self.factors(&tx, account.id).await?;

			if !factors.is_empty() {
				let challenge = self
					.second_factor_challenge(&tx, account.id, factors, None)
					.await?;

				return Ok(SignInStep::SecondFactor(challenge));
			}

			let now = leptoskit_db::now();
			let sign_in = self
				.start_session(&tx, account.id, Aal::One, now, None, request)
				.await?;
			record(&tx, Some(account.id), EventKind::SignedIn, request).await?;
			Ok(SignInStep::SignedIn(Box::new(sign_in)))
		}
		.await;
		finish(tx, result).await
	}

	/// The fields of a new message, bound to the origin's host.
	fn siws_input(
		&self,
		statement: &str,
		request_id: &str,
		address: Option<&Pubkey>,
	) -> Result<SiwsInput> {
		#[cfg(feature = "solana")]
		{
			self.signed_input(statement, request_id, address)
		}
		#[cfg(not(feature = "solana"))]
		{
			let _ = (self, statement, request_id, address);
			Err(AuthError::Unsupported(
				"wallets need leptoskit_auth's `solana` feature",
			))
		}
	}

	#[cfg_attr(
		not(feature = "solana"),
		allow(dead_code, reason = "only the `solana` feature issues messages")
	)]
	fn signed_input(
		&self,
		statement: &str,
		request_id: &str,
		address: Option<&Pubkey>,
	) -> Result<SiwsInput> {
		let origin = self.inner.origin.as_deref().ok_or_else(|| {
			AuthError::Config("wallets need `[auth] origin`, whose host messages name".into())
		})?;
		let domain = origin
			.split_once("://")
			.map(|(_, authority)| authority)
			.filter(|domain| crate::siws::valid_domain(domain))
			.ok_or_else(|| AuthError::Config("`[auth] origin` has no host".into()))?;
		let now = leptoskit_db::now();
		let nonce = leptoskit_db::base58::encode(&crate::secrets::random::<16>()?);
		let time =
			|at: leptoskit_db::Timestamp| at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

		Ok(SiwsInput {
			domain: domain.to_owned(),
			address: address.map(ToString::to_string),
			statement: Some(statement.to_owned()),
			uri: Some(origin.to_owned()),
			version: Some("1".to_owned()),
			chain_id: Some(self.inner.solana.chain_id.clone()),
			// 128 random bits in base58: at least 16 letters and digits, as
			// SIWS asks.
			nonce: Some(nonce),
			issued_at: Some(time(now)),
			expiration_time: Some(time(now + CEREMONY_LIFETIME)),
			not_before: None,
			request_id: Some(request_id.to_owned()),
			resources: Vec::new(),
		})
	}

	/// The app's name as a statement can hold it.
	fn statement_name(&self) -> String {
		statement_name(&self.inner.app_name)
	}

	/// Keeps `input` sealed until its wallet answers, bound to `subject`.
	async fn issue(&self, purpose: Purpose, subject: &str, input: &SiwsInput) -> Result<()> {
		let state = serde_json::to_vec(input)
			.map_err(|_| AuthError::Config("a challenge couldn't be stored".into()))?;
		self.store_ceremony(&self.inner.db, purpose, subject, &state)
			.await?;
		Ok(())
	}

	/// Uses up the challenge: from here it's spent, whatever the answer.
	async fn claim(
		&self,
		purpose: Purpose,
		subject: &str,
		token: Option<&SessionToken>,
	) -> Result<SiwsInput> {
		let state = self
			.take_ceremony(purpose, subject, token)
			.await?
			.ok_or(AuthError::WalletFailed)?;
		serde_json::from_slice(&state).map_err(|_| AuthError::WalletFailed)
	}

	/// Checks `proof` against the message `issued` described, recording a
	/// refusal in the audit trail.
	async fn checked(
		&self,
		issued: &SiwsInput,
		proof: &WalletProof,
		account: Option<Id<Account>>,
		request: &RequestInfo,
	) -> Result<()> {
		if verify(issued, proof, leptoskit_db::now()).is_err() {
			record(&self.inner.db, account, EventKind::WalletFailed, request).await?;

			return Err(AuthError::WalletFailed);
		}

		Ok(())
	}

	/// Keeps a fresh proof for a linked wallet.
	async fn proved(
		&self,
		wallet: &Wallet,
		proof: &WalletProof,
		request: &RequestInfo,
	) -> Result<WalletInfo> {
		let tx = self.inner.db.begin().await?;
		let result = async {
			set_proof(&tx, wallet, proof).await?;
			record(
				&tx,
				Some(wallet.account_id),
				EventKind::WalletProved,
				request,
			)
			.await?;
			Ok(Wallet::get(&tx, wallet.id).await?)
		}
		.await;
		Ok(finish(tx, result).await?.into())
	}

	async fn limit_wallet_start(&self, request: &RequestInfo) -> Result<()> {
		match request.ip_text() {
			Some(ip) => {
				self.limit(
					&[b"ceremony-ip", ip.as_bytes()],
					self.inner.limits.ceremony_per_ip,
				)
				.await
			}
			None => Ok(()),
		}
	}
}

/// Why `proof` isn't an answer to the message `issued` described, if it
/// isn't: the text must parse as Sign-In With Solana, be exactly the message
/// issued for the address that signed, not have expired, and carry a valid
/// ed25519 signature by that address.
pub(crate) fn verify(
	issued: &SiwsInput,
	proof: &WalletProof,
	now: leptoskit_db::Timestamp,
) -> std::result::Result<(), &'static str> {
	if issued
		.address
		.as_deref()
		.is_some_and(|address| address != proof.address.as_inner())
	{
		return Err("another wallet was asked to sign");
	}

	let parsed = SiwsMessage::parse(&proof.message).map_err(|_| "not a SIWS message")?;

	if parsed.address != proof.address {
		return Err("the message names another address");
	}

	let expected = issued.message(&proof.address);

	if !bool::from(expected.as_bytes().ct_eq(proof.message.as_bytes())) {
		return Err("not the message issued");
	}

	let expires = issued
		.expiration_time
		.as_deref()
		.and_then(|time| chrono::DateTime::parse_from_rfc3339(time).ok())
		.ok_or("no expiry")?;

	if now >= expires {
		return Err("expired");
	}

	signature(proof)
}

#[cfg(feature = "solana")]
fn signature(proof: &WalletProof) -> std::result::Result<(), &'static str> {
	let key = ed25519_dalek::VerifyingKey::from_bytes(&proof.address.to_bytes())
		.map_err(|_| "not an ed25519 key")?;
	let signature = ed25519_dalek::Signature::from_bytes(&proof.signature.to_bytes());
	// Strict: refuses small-order keys and non-canonical signatures, so a
	// signature can't be reshaped into a second valid one.
	key.verify_strict(proof.message.as_bytes(), &signature)
		.map_err(|_| "bad signature")
}

#[cfg(not(feature = "solana"))]
fn signature(_proof: &WalletProof) -> std::result::Result<(), &'static str> {
	Err("wallets need the `solana` feature")
}

/// An app's name as a statement can hold it: one line, no padding, at most
/// 100 characters, and "this app" when there's nothing left.
fn statement_name(app_name: &str) -> String {
	let name: String = app_name
		.chars()
		.map(|character| {
			if character.is_control() {
				' '
			} else {
				character
			}
		})
		.take(100)
		.collect();
	let name = name.trim();

	if name.is_empty() {
		"this app".to_owned()
	} else {
		name.to_owned()
	}
}

fn sign_in_off() -> AuthError {
	AuthError::Unsupported("signing in with wallets is off on this server")
}

/// The subject of linking a wallet from `session`.
fn link_subject(session: Id<crate::Session>) -> String {
	format!("wallet-link:{session}")
}

/// The subject of re-proving a wallet from `session`.
fn proof_subject(session: Id<crate::Session>) -> String {
	format!("wallet-proof:{session}")
}

/// A label as typed: trimmed and at most 100 characters; `None` when empty.
fn label(text: &str) -> Option<String> {
	let text = text.trim();
	(!text.is_empty()).then(|| text.chars().take(LABEL_LENGTH).collect())
}

/// The wallet at `address`, whoever's it is.
async fn wallet_at(connection: &impl Connection, address: &Pubkey) -> Result<Option<Wallet>> {
	Ok(Wallet::query()
		.filter(|wallet| wallet.chain.eq(CHAIN.to_owned()) & wallet.address.eq(address))
		.first(connection)
		.await?)
}

/// `account`'s wallet `id`.
async fn owned(
	connection: &impl Connection,
	account: Id<Account>,
	id: Id<Wallet>,
) -> Result<Wallet> {
	Wallet::query()
		.filter(|wallet| wallet.id.eq(id) & wallet.account_id.eq(account))
		.first(connection)
		.await?
		.ok_or(AuthError::NotFound)
}

/// Keeps `proof` as the wallet's latest, proved now.
async fn set_proof(
	connection: &impl Connection,
	wallet: &Wallet,
	proof: &WalletProof,
) -> Result<()> {
	Wallet::update()
		.filter(|row| row.id.eq(wallet.id))
		.set(|row| row.last_proved_at, leptoskit_db::now())
		.set(|row| row.proof, proof.message.clone())
		.set(|row| row.proof_signature, proof.signature.clone())
		.run(connection)
		.await?;
	Ok(())
}

#[cfg(all(test, feature = "solana"))]
mod tests {
	use ed25519_dalek::Signer;
	use ed25519_dalek::SigningKey;
	use leptoskit_db::Signature;

	use super::*;

	fn key() -> SigningKey {
		SigningKey::from_bytes(&crate::secrets::random::<32>().unwrap())
	}

	fn issued() -> SiwsInput {
		SiwsInput {
			domain: "notes.example".into(),
			statement: Some("Sign in to Notes.".into()),
			uri: Some("https://notes.example".into()),
			version: Some("1".into()),
			chain_id: Some("mainnet".into()),
			nonce: Some("4WQ5WmPxpHdRqv8a3NhVF2".into()),
			issued_at: Some("2026-10-30T12:00:00Z".into()),
			expiration_time: Some("2026-10-30T12:05:00Z".into()),
			request_id: Some("sign-in".into()),
			..SiwsInput::default()
		}
	}

	fn sign(key: &SigningKey, message: &str) -> WalletProof {
		WalletProof {
			address: Pubkey::from_bytes(key.verifying_key().to_bytes()),
			message: message.to_owned(),
			signature: Signature::from_bytes(key.sign(message.as_bytes()).to_bytes()),
		}
	}

	fn at(time: &str) -> leptoskit_db::Timestamp {
		chrono::DateTime::parse_from_rfc3339(time)
			.unwrap()
			.with_timezone(&chrono::Utc)
	}

	#[test]
	fn the_issued_message_signed_by_its_address_checks_out() {
		let key = key();
		let address = Pubkey::from_bytes(key.verifying_key().to_bytes());
		let proof = sign(&key, &issued().message(&address));
		assert_eq!(
			verify(&issued(), &proof, at("2026-10-30T12:01:00Z")),
			Ok(())
		);
	}

	#[test]
	fn anything_but_the_issued_message_from_its_signer_is_refused() {
		let key = key();
		let other = self::key();
		let address = Pubkey::from_bytes(key.verifying_key().to_bytes());
		let message = issued().message(&address);
		let now = at("2026-10-30T12:01:00Z");

		// Expired.
		assert_eq!(
			verify(&issued(), &sign(&key, &message), at("2026-10-30T12:05:00Z")),
			Err("expired")
		);
		// Another site's, or another nonce: not the message issued.
		for changed in [
			message.replace("notes.example wants", "evil.example wants"),
			message.replace("4WQ5WmPxpHdRqv8a3NhVF2", "4WQ5WmPxpHdRqv8a3NhVF3"),
			message.replace("Request ID: sign-in", "Request ID: link"),
		] {
			assert_eq!(
				verify(&issued(), &sign(&key, &changed), now),
				Err("not the message issued")
			);
		}
		// Signed by someone else, claiming this address.
		let mut forged = sign(&other, &message);
		forged.address = address.clone();
		assert_eq!(verify(&issued(), &forged, now), Err("bad signature"));
		// Another address's message, signed by it, presented as this one's.
		let theirs = sign(
			&other,
			&issued().message(&Pubkey::from_bytes(other.verifying_key().to_bytes())),
		);
		let mut mixed = theirs.clone();
		mixed.address = address.clone();

		assert_eq!(
			verify(&issued(), &mixed, now),
			Err("the message names another address")
		);
		// A proof asked of one wallet, answered by another.
		let asked = SiwsInput {
			address: Some(address.to_string()),
			..issued()
		};
		assert_eq!(
			verify(&asked, &theirs, now),
			Err("another wallet was asked to sign")
		);
		// Garbage.
		let mut garbage = sign(&key, "hello");
		garbage.address = address;
		assert_eq!(verify(&issued(), &garbage, now), Err("not a SIWS message"));
	}

	#[test]
	fn statements_name_the_app_on_one_line() {
		assert_eq!(statement_name("Notes"), "Notes");
		assert_eq!(statement_name(" Notes\nNow\t"), "Notes Now");
		assert_eq!(statement_name("\n\t"), "this app");
		assert_eq!(statement_name(&"a".repeat(300)).len(), 100);
		assert!(crate::siws::valid_statement(&format!(
			"Sign in to {}.",
			statement_name("Notes\r\nInc")
		)));
	}

	#[test]
	fn labels_are_trimmed_and_bounded() {
		assert_eq!(label("  Treasury "), Some("Treasury".into()));
		assert_eq!(label("   "), None);
		assert_eq!(label(&"é".repeat(300)).unwrap().chars().count(), 100);
	}
}
