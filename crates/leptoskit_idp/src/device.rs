//! The device authorization flow (RFC 8628), for CLIs, TVs and anything
//! else without a browser: the device shows a short code, someone enters it
//! on their phone or computer, signed in, and approves; meanwhile the device
//! polls the token endpoint until tokens arrive.

use std::time::Duration;

use leptoskit_auth::Limit;
use leptoskit_auth::RequestInfo;
use leptoskit_auth::SignedIn;
use leptoskit_db::Json;

use crate::AppInfo;
use crate::Client;
use crate::ClientCredentials;
use crate::DeviceAuthorization;
use crate::DeviceCode;
use crate::DeviceRequest;
use crate::GrantType;
use crate::Idp;
use crate::IdpError;
use crate::NewDeviceCode;
use crate::Result;
use crate::TokenRequest;
use crate::TokenResponse;
use crate::authorize::with_query;
use crate::secrets;
use crate::tokens::Extras;
use crate::tokens::finish;
use crate::tokens::required;
use crate::tokens::split_scopes;
use crate::tokens::usable;

/// Seconds a device waits between polls, until it's told to slow down.
const INTERVAL: i32 = 5;

/// How much longer a device that polls too fast must wait (RFC 8628 §3.5).
const SLOW_DOWN: i32 = 5;

/// How often one person may look up or answer a code: guessing someone
/// else's code means guessing one of 20⁸.
const LOOKUPS: Limit = Limit::new(20, Duration::from_mins(15));

impl Idp {
	/// Answers `POST /oauth/device_authorization`: starts a sign-in for a
	/// device, which shows the user code and polls `/oauth/token` with the
	/// device code.
	///
	/// # Errors
	///
	/// `invalid_client` when the app doesn't authenticate as it registered;
	/// `unauthorized_client` when it isn't registered for the device flow;
	/// `invalid_scope` for a scope it may not ask for; or when the database
	/// fails.
	pub async fn authorize_device(
		&self,
		credentials: &ClientCredentials,
		scope: Option<&str>,
	) -> Result<DeviceAuthorization> {
		let client = self.authenticate(credentials).await?;

		if !client.grant_types.contains(&GrantType::DeviceCode) {
			return Err(IdpError::oauth(
				"unauthorized_client",
				"this app isn't registered for the device flow",
			));
		}

		let scopes = split_scopes(scope.unwrap_or_default());
		if scopes.is_empty() {
			return Err(IdpError::invalid_scope("ask for at least one scope"));
		}
		if let Some(scope) = scopes
			.iter()
			.find(|scope| !client.scopes.contains(*scope) || !self.knows_scope(scope))
		{
			return Err(IdpError::invalid_scope(format!(
				"this app may not ask for `{scope}`"
			)));
		}

		let now = leptoskit_db::now();
		let lifetime = Self::lifetime(self.inner.lifetimes.device_code)?;
		// Codes are kept a day past expiry, so a replayed one still ends what
		// it started.
		DeviceCode::query()
			.filter(|row| row.expires_at.lt(now - chrono::TimeDelta::days(1)))
			.delete(&self.inner.db)
			.await?;

		// User codes are short, so two pending ones may clash: try again.
		for _ in 0..4 {
			let device_code = secrets::token()?;
			let user_code = secrets::user_code()?;
			let normalized = secrets::normalize_user_code(&user_code)
				.ok_or_else(|| IdpError::Config("a user code didn't read back".into()))?;
			let inserted = NewDeviceCode {
				client_id: client.id,
				device_hash: secrets::hash(&device_code),
				user_code_hash: secrets::hash(&normalized),
				scopes: Json(scopes.clone()),
				account_id: None,
				auth_time: None,
				approved: None,
				interval: INTERVAL,
				polled_at: None,
				expires_at: now + lifetime,
				used_at: None,
				family_id: None,
			}
			.insert(&self.inner.db)
			.await;

			match inserted {
				Ok(_) => {
					let verification_uri =
						format!("{}{}", self.inner.issuer, self.inner.device_page);
					return Ok(DeviceAuthorization {
						device_code,
						verification_uri_complete: with_query(
							&verification_uri,
							&[("user_code", user_code.as_str())],
						),
						user_code,
						verification_uri,
						expires_in: u64::try_from(lifetime.num_seconds()).unwrap_or_default(),
						interval: INTERVAL.unsigned_abs().into(),
					});
				}
				Err(leptoskit_db::DbError::Conflict { .. }) => {}
				Err(error) => return Err(error.into()),
			}
		}

		Err(IdpError::Config("couldn't pick a user code".into()))
	}

	/// What the device page shows for the code `me` typed: the app, and what
	/// it asks to do.
	///
	/// # Errors
	///
	/// [`IdpError::NotFound`] when no device is waiting for that code;
	/// [`IdpError::Auth`] when `me` has guessed too many codes lately; or
	/// when the database fails.
	pub async fn device_request(&self, me: &SignedIn, user_code: &str) -> Result<DeviceRequest> {
		let (device, client) = self.waiting_device(me, user_code).await?;
		Ok(DeviceRequest {
			app: AppInfo {
				client_id: client.client_id,
				name: client.name,
			},
			scopes: self.describe(&device.scopes),
			user_code: secrets::normalize_user_code(user_code)
				.map(|code| format!("{}-{}", &code[..4], &code[4..]))
				.unwrap_or_default(),
		})
	}

	/// Answers a device's request as `me`: approving it lets the device's
	/// next poll have tokens, and records the grant like consent does;
	/// refusing it tells the device `access_denied`.
	///
	/// # Errors
	///
	/// As [`Idp::device_request`].
	pub async fn decide_device(
		&self,
		me: &SignedIn,
		user_code: &str,
		approve: bool,
		info: &RequestInfo,
	) -> Result<()> {
		let (device, client) = self.waiting_device(me, user_code).await?;
		let tx = self.inner.db.begin().await?;
		let result = async {
			let answered = DeviceCode::update()
				.filter(|row| row.id.eq(device.id) & row.approved.is_null())
				.set(|row| row.approved, Some(approve))
				.set(|row| row.account_id, Some(me.account))
				.set(|row| row.auth_time, Some(me.authenticated_at))
				.run(&tx)
				.await?;

			if answered == 0 {
				return Err(IdpError::NotFound);
			}
			if approve {
				self.grant(&tx, me, &client, &device.scopes, info).await?;
			}
			Ok(())
		}
		.await;

		finish(tx, result).await
	}

	/// The device waiting for `user_code`, counting the lookup against
	/// `me`'s limit.
	async fn waiting_device(&self, me: &SignedIn, user_code: &str) -> Result<(DeviceCode, Client)> {
		self.inner
			.auth
			.limit_attempts("idp: device code", &me.account.to_string(), LOOKUPS)
			.await?;

		let normalized = secrets::normalize_user_code(user_code).ok_or(IdpError::NotFound)?;
		let now = leptoskit_db::now();
		let device = DeviceCode::query()
			.filter(|row| {
				row.user_code_hash.eq(secrets::hash(&normalized))
					& row.approved.is_null()
					& row.expires_at.gt(now)
			})
			.first(&self.inner.db)
			.await?
			.ok_or(IdpError::NotFound)?;
		let client = Client::find(&self.inner.db, device.client_id)
			.await?
			.ok_or(IdpError::NotFound)?;
		Ok((device, client))
	}

	/// Answers a device's poll of `/oauth/token`: tokens once someone
	/// approved, and otherwise why not yet.
	pub(crate) async fn exchange_device_code(
		&self,
		request: &TokenRequest,
		client: &Client,
		info: &RequestInfo,
	) -> Result<TokenResponse> {
		let presented = required(request.device_code.as_ref(), "device_code")?;
		let device = DeviceCode::query()
			.filter(|row| row.device_hash.eq(secrets::hash(presented)))
			.first(&self.inner.db)
			.await?
			.filter(|row| row.client_id == client.id)
			.ok_or_else(|| IdpError::invalid_grant("that device code isn't valid"))?;
		let now = leptoskit_db::now();

		if device.used_at.is_some() {
			// A device code trades once; seen again, it was copied.
			if let Some(family) = device.family_id {
				self.end_reused(&self.inner.db, family, device.account_id, client, info)
					.await?;
			}
			return Err(IdpError::invalid_grant("that device code was already used"));
		}
		if device.expires_at <= now {
			return Err(IdpError::oauth(
				"expired_token",
				"the code expired before anyone entered it; start again",
			));
		}

		let too_soon = device
			.polled_at
			.is_some_and(|polled| (now - polled).num_seconds() < i64::from(device.interval));
		let mut poll = DeviceCode::update().filter(|row| row.id.eq(device.id));
		poll = match device.polled_at {
			Some(polled) => poll.filter(|row| row.polled_at.eq(Some(polled))),
			None => poll.filter(|row| row.polled_at.is_null()),
		};
		poll = poll.set(|row| row.polled_at, Some(now));
		if too_soon {
			poll = poll.set(|row| row.interval, device.interval + SLOW_DOWN);
		}
		// Two polls at once: the one that lost is too soon.
		if poll.run(&self.inner.db).await? == 0 || too_soon {
			return Err(IdpError::oauth(
				"slow_down",
				format!("poll at most every {} seconds", device.interval + SLOW_DOWN),
			));
		}

		let (Some(approved), Some(account), Some(auth_time)) =
			(device.approved, device.account_id, device.auth_time)
		else {
			return Err(IdpError::oauth(
				"authorization_pending",
				"nobody has entered the code yet",
			));
		};
		if !approved {
			return Err(IdpError::oauth("access_denied", "the person said no"));
		}

		let signers = self.signers(client).await?;
		let tx = self.inner.db.begin().await?;
		let result = async {
			let used = DeviceCode::update()
				.filter(|row| row.id.eq(device.id) & row.used_at.is_null())
				.set(|row| row.used_at, Some(now))
				.run(&tx)
				.await?;

			if used == 0 {
				return Err(IdpError::invalid_grant("that device code was already used"));
			}
			if !usable(&tx, account).await? {
				return Err(IdpError::invalid_grant("that account can't sign in"));
			}

			let family = self
				.start_family(
					&tx,
					client,
					account,
					device.scopes.0.clone(),
					Vec::new(),
					auth_time,
					request.dpop_jkt.as_deref(),
				)
				.await?;
			DeviceCode::update()
				.filter(|row| row.id.eq(device.id))
				.set(|row| row.family_id, Some(family.id))
				.run(&tx)
				.await?;

			self.issue(
				&tx,
				&signers,
				client,
				&family,
				&family.scopes,
				Extras {
					refresh: true,
					dpop: request.dpop_jkt.as_deref(),
					..Extras::default()
				},
			)
			.await
		}
		.await;

		finish(tx, result).await
	}
}
