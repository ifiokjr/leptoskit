//! Rotating the server's keys: re-encrypting what's stored under the
//! newest one, so an old key can be retired.

use crate::Auth;
use crate::Challenge;
use crate::Keys;
use crate::ProviderToken;
use crate::Result;
use crate::TotpSecret;
use crate::provider_tokens::token_context;

/// What [`Auth::rotate_keys`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rotated {
	/// Authenticator secrets re-encrypted under the newest key.
	pub totp: u64,
	/// Unfinished passkey, provider, and merge ceremonies re-encrypted.
	pub ceremonies: u64,
	/// Providers' stored tokens re-encrypted (each access and refresh
	/// token counts).
	pub provider_tokens: u64,
	/// Encrypted values already under the newest key.
	pub current: u64,
}

impl Auth {
	/// Re-encrypts every stored secret under the newest key in
	/// `AUTH_ENCRYPTION_KEYS`: authenticator secrets, the state of
	/// ceremonies still in flight, and providers' stored tokens. Each row is
	/// replaced only if nobody changed it meanwhile, so it's safe while the
	/// app runs, and running it again does nothing.
	///
	/// Hashes can't be re-keyed: recovery codes and emailed codes made
	/// under an older key keep working only while that key is listed. Keep
	/// it until the people with recovery codes have made new ones, or
	/// accept that their old codes stop working.
	///
	/// # Errors
	///
	/// When the database fails, or a value names a key that's no longer
	/// listed.
	pub async fn rotate_keys(&self) -> Result<Rotated> {
		let db = &self.inner.db;
		let keys = &self.inner.keys;
		let mut rotated = Rotated::default();

		for row in TotpSecret::query().all(db).await? {
			if Keys::sealed_with(&row.secret) == Some(keys.newest()) {
				rotated.current += 1;
				continue;
			}

			let account = row.account_id.uuid();
			let context = crate::mfa::totp_context(account.as_bytes());
			let resealed = keys.seal(&context, &keys.open(&context, &row.secret)?)?;
			rotated.totp += TotpSecret::update()
				.filter(|update| update.id.eq(row.id) & update.secret.eq(row.secret.clone()))
				.set(|update| update.secret, resealed)
				.run(db)
				.await?;
		}

		let now = leptoskit_db::now();
		let live = Challenge::query()
			.filter(|row| row.used_at.is_null() & row.expires_at.gt(now) & row.state.is_not_null())
			.all(db)
			.await?;

		for row in live {
			let Some(sealed) = &row.state else {
				continue;
			};

			if Keys::sealed_with(sealed) == Some(keys.newest()) {
				rotated.current += 1;
				continue;
			}

			let context =
				crate::passkeys::ceremony_context(row.purpose, &row.subject, &row.secret_hash);
			let resealed = keys.seal(&context, &keys.open(&context, sealed)?)?;
			rotated.ceremonies += Challenge::update()
				.filter(|update| update.id.eq(row.id) & update.state.eq(Some(sealed.clone())))
				.set(|update| update.state, Some(resealed))
				.run(db)
				.await?;
		}

		self.rotate_provider_tokens(&mut rotated).await?;
		Ok(rotated)
	}

	async fn rotate_provider_tokens(&self, rotated: &mut Rotated) -> Result<()> {
		let db = &self.inner.db;
		let keys = &self.inner.keys;

		for row in ProviderToken::query().all(db).await? {
			let id = row.identity_id.uuid();
			let reseal = |sealed: &[u8], which: &[u8]| -> Result<Option<Vec<u8>>> {
				if Keys::sealed_with(sealed) == Some(keys.newest()) {
					return Ok(None);
				}

				let context = token_context(id.as_bytes(), which);
				Ok(Some(keys.seal(&context, &keys.open(&context, sealed)?)?))
			};
			let access = reseal(&row.access_token, b"access")?;
			let refresh = row
				.refresh_token
				.as_deref()
				.map(|sealed| reseal(sealed, b"refresh"))
				.transpose()?
				.flatten();
			let count = u64::from(access.is_some()) + u64::from(refresh.is_some());
			rotated.current += 1 + u64::from(row.refresh_token.is_some()) - count;

			if count == 0 {
				continue;
			}

			// Only if nobody replaced them meanwhile, such as a refresh.
			let changed = ProviderToken::update()
				.filter(|update| update.id.eq(row.id) & update.updated_at.eq(row.updated_at))
				.set(
					|update| update.access_token,
					access.unwrap_or(row.access_token.clone()),
				)
				.set(
					|update| update.refresh_token,
					refresh.or(row.refresh_token.clone()),
				)
				.run(db)
				.await?;
			rotated.provider_tokens += changed * count;
		}

		Ok(())
	}
}
