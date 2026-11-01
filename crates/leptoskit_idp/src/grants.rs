//! Connected apps, which people see and disconnect, and signing out from an
//! app that signed them in (OpenID Connect RP-Initiated Logout).

use leptoskit_auth::EventKind;
use leptoskit_auth::RequestInfo;
use leptoskit_auth::SignedIn;
use leptoskit_db::Id;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

use crate::AppInfo;
use crate::Authorization;
use crate::Client;
use crate::ConnectedApp;
use crate::DeviceCode;
use crate::Family;
use crate::Grant;
use crate::Idp;
use crate::IdpError;
use crate::Result;
use crate::authorize::with_query;
use crate::jwt::ID_TOKEN_TYPE;
use crate::tokens::revoke_family;

/// The query of `GET /oauth/end_session`. Its `Debug` leaves out the ID
/// token.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndSessionRequest {
	/// An ID token this provider gave the app, proving who it signed in.
	#[serde(default)]
	pub id_token_hint: Option<String>,
	/// Where to send the person afterwards: one of the app's registered
	/// post-logout redirect URIs.
	#[serde(default)]
	pub post_logout_redirect_uri: Option<String>,
	/// Echoed back to the app.
	#[serde(default)]
	pub state: Option<String>,
	/// The app, when there's no ID token to name it.
	#[serde(default)]
	pub client_id: Option<String>,
}

impl std::fmt::Debug for EndSessionRequest {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("EndSessionRequest")
			.field("id_token_hint", &self.id_token_hint.as_ref().map(|_| "…"))
			.field("post_logout_redirect_uri", &self.post_logout_redirect_uri)
			.field("state", &self.state)
			.field("client_id", &self.client_id)
			.finish()
	}
}

/// What `GET /oauth/end_session` did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EndSession {
	/// Where to send the browser.
	pub redirect_to: String,
	/// Whether the browser's own session ended, so its cookie should go.
	pub signed_out: bool,
}

impl Idp {
	/// The apps `me` has connected, most recently connected first.
	///
	/// # Errors
	///
	/// When the database fails.
	pub async fn connected_apps(&self, me: &SignedIn) -> Result<Vec<ConnectedApp>> {
		let grants = Grant::query()
			.filter(|grant| grant.account_id.eq(me.account))
			.order_by(|grant| grant.created_at.desc())
			.all(&self.inner.db)
			.await?;
		let mut apps = Vec::with_capacity(grants.len());

		for grant in grants {
			let Some(client) = Client::find(&self.inner.db, grant.client_id).await? else {
				continue;
			};
			apps.push(ConnectedApp {
				app: AppInfo {
					client_id: client.client_id,
					name: client.name,
				},
				scopes: self.describe(&grant.scopes),
				connected_at: grant.created_at,
				last_used_at: grant.last_used_at,
			});
		}

		Ok(apps)
	}

	/// Disconnects an app from `me`'s account: its grant goes, and with it
	/// every refresh token it holds and every access token checked against
	/// this server, and their audit trail says so. Returns whether it was
	/// connected.
	///
	/// # Errors
	///
	/// When the database fails.
	pub async fn disconnect(
		&self,
		me: &SignedIn,
		client_id: &str,
		info: &RequestInfo,
	) -> Result<bool> {
		let Some(client) = self.client(client_id).await? else {
			return Ok(false);
		};
		let tx = self.inner.db.begin().await?;
		let result = async {
			// Codes not yet traded go too, so none outlives the grant.
			Authorization::query()
				.filter(|row| row.account_id.eq(me.account) & row.client_id.eq(client.id))
				.delete(&tx)
				.await?;
			// Devices still waiting for a code go too.
			DeviceCode::query()
				.filter(|row| row.account_id.eq(me.account) & row.client_id.eq(client.id))
				.delete(&tx)
				.await?;
			let removed = Grant::query()
				.filter(|grant| grant.account_id.eq(me.account) & grant.client_id.eq(client.id))
				.delete(&tx)
				.await?;

			if removed > 0 {
				self.inner
					.auth
					.record(
						&tx,
						Some(me.account),
						EventKind::AppDisconnected,
						Some(&client.name),
						info,
					)
					.await?;
			}

			Ok(removed > 0)
		}
		.await;

		crate::tokens::finish(tx, result).await
	}

	/// Answers `GET /oauth/end_session`: an app signing someone out.
	///
	/// With an ID token from this provider, the app's sign-in ends: its
	/// refresh-token family is revoked. If the browser is still signed in as
	/// the same person, its session ends too. Without one, nobody is signed
	/// out, since anyone can link here. Either way the browser goes to the
	/// app's registered `post_logout_redirect_uri`, with `state`, or to your
	/// signed-out page.
	///
	/// # Errors
	///
	/// [`IdpError::OAuth`] (`invalid_request`) when the ID token isn't one
	/// this provider issued, or the redirect isn't registered for the app;
	/// or when the database fails.
	pub async fn end_session(
		&self,
		request: &EndSessionRequest,
		signed_in: Option<&SignedIn>,
		info: &RequestInfo,
	) -> Result<EndSession> {
		let hint = match request.id_token_hint.as_deref() {
			Some(token) => {
				Some(self.id_token_hint(token).await?.ok_or_else(|| {
					IdpError::invalid_request("id_token_hint isn't an ID token from this provider")
				})?)
			}
			None => None,
		};

		let client_id = match (&hint, request.client_id.as_deref()) {
			(Some(hint), Some(client_id)) if hint.client_id != client_id => {
				return Err(IdpError::invalid_request(
					"client_id doesn't match the ID token",
				));
			}

			(Some(hint), _) => Some(hint.client_id.clone()),
			(None, client_id) => client_id.map(str::to_owned),
		};

		let client = match &client_id {
			Some(client_id) => self.client(client_id).await?,
			None => None,
		};

		let redirect_to = match (&request.post_logout_redirect_uri, &client) {
			(Some(uri), Some(client)) if client.post_logout_redirect_uris.contains(uri) => {
				match &request.state {
					Some(state) => with_query(uri, &[("state", state)]),
					None => uri.clone(),
				}
			}

			(Some(_), _) => {
				return Err(IdpError::invalid_request(
					"post_logout_redirect_uri isn't registered for this app",
				));
			}
			(None, _) => self.inner.signed_out_page.clone(),
		};

		let mut signed_out = false;

		if let Some(hint) = hint {
			let family = match hint.family {
				Some(family) => Family::find(&self.inner.db, family).await?,
				None => None,
			};

			if let Some(family) = family {
				revoke_family(&self.inner.db, family.id).await?;

				if let Some(me) = signed_in.filter(|me| Some(me.account) == family.account_id) {
					signed_out = self.inner.auth.revoke(me, me.session, info).await?;
				}
			}
		}

		Ok(EndSession {
			redirect_to,
			signed_out,
		})
	}

	/// The app and sign-in an ID token names, if this provider issued it.
	/// It may have expired: an app signs people out long after it got one.
	async fn id_token_hint(&self, token: &str) -> Result<Option<Hint>> {
		let Some(claims) = self.verified_claims(token, ID_TOKEN_TYPE).await? else {
			return Ok(None);
		};

		if claims.get("iss").and_then(Value::as_str) != Some(self.inner.issuer.as_str()) {
			return Ok(None);
		}

		let Some(client_id) = claims.get("aud").and_then(Value::as_str) else {
			return Ok(None);
		};

		Ok(Some(Hint {
			client_id: client_id.to_owned(),
			family: claims
				.get("sid")
				.and_then(Value::as_str)
				.and_then(|sid| Id::parse(sid).ok()),
		}))
	}
}

struct Hint {
	client_id: String,
	family: Option<Id<Family>>,
}
