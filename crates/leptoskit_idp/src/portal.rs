//! The developer portal: signed-in people register and manage apps of their
//! own that sign in with yours, for third-party developers. Each person sees
//! only their own apps; none is first-party, and none may introspect other
//! apps' tokens.

use leptoskit_auth::EventKind;
use leptoskit_auth::RequestInfo;
use leptoskit_auth::SignedIn;
use leptoskit_db::Connection;
use leptoskit_db::Json;
use leptoskit_db::Tx;

use crate::AppChanges;
use crate::Client;
use crate::ClientInfo;
use crate::Family;
use crate::Grant;
use crate::Idp;
use crate::IdpError;
use crate::NewApp;
use crate::NewClientSpec;
use crate::RegisteredApp;
use crate::Result;
use crate::SigningAlgorithm;
use crate::clients::check_authentication;
use crate::clients::dedupe;
use crate::clients::metadata;
use crate::tokens::finish;
use crate::tokens::revoke_family;

impl Idp {
	/// Whether the developer portal is on.
	pub fn has_developer_portal(&self) -> bool {
		self.inner.developer_portal.is_some()
	}

	fn portal(&self) -> Result<crate::Portal> {
		self.inner.developer_portal.ok_or(IdpError::NotFound)
	}

	/// The apps `me` registered, oldest first.
	///
	/// # Errors
	///
	/// [`IdpError::NotFound`] when the portal is off, or when the database
	/// fails.
	pub async fn my_apps(&self, me: &SignedIn) -> Result<Vec<ClientInfo>> {
		self.portal()?;
		let apps = Client::query()
			.filter(|client| client.owner_id.eq(Some(me.account)))
			.order_by(|client| client.created_at.asc())
			.all(&self.inner.db)
			.await?;
		Ok(apps.into_iter().map(ClientInfo::from).collect())
	}

	/// Registers an app owned by `me`, and returns it with its secret, shown
	/// this once.
	///
	/// # Errors
	///
	/// [`IdpError::NotFound`] when the portal is off; `invalid_client_metadata`
	/// when `me` already has as many apps as the portal allows, or the app
	/// isn't one that may register; or when the database fails.
	pub async fn register_app(
		&self,
		me: &SignedIn,
		app: NewApp,
		info: &RequestInfo,
	) -> Result<RegisteredApp> {
		let portal = self.portal()?;
		let tx = self.inner.db.begin().await?;
		let result = async {
			let owned = Client::query()
				.filter(|client| client.owner_id.eq(Some(me.account)))
				.count(&tx)
				.await?;

			if owned >= u64::from(portal.apps_per_person) {
				return Err(metadata(format!(
					"you can register at most {} apps; delete one first",
					portal.apps_per_person
				)));
			}

			let spec = NewClientSpec {
				name: app.name,
				redirect_uris: app.redirect_uris,
				post_logout_redirect_uris: app.post_logout_redirect_uris,
				scopes: app.scopes,
				auth_method: app.auth_method,
				grant_types: app.grant_types,
				jwks: app.jwks,
				id_token_alg: SigningAlgorithm::Es256,
				first_party: false,
				introspection: false,
				dpop_bound: false,
			};
			let (client, secret) = self
				.insert_client(spec, Some(me.account), None, &tx)
				.await?;
			self.inner
				.auth
				.record(
					&tx,
					Some(me.account),
					EventKind::AppRegistered,
					Some(&client.name),
					info,
				)
				.await?;
			Ok(RegisteredApp {
				app: client,
				secret: secret.map(|secret| secret.expose().to_owned()),
			})
		}
		.await;

		finish(tx, result).await
	}

	/// Changes an app `me` owns. Taking scopes away also takes them from
	/// everyone who connected it: their grants keep only the scopes left, and
	/// the sign-ins that carried the old ones end.
	///
	/// # Errors
	///
	/// [`IdpError::NotFound`] when the portal is off or `me` owns no such
	/// app; `invalid_client_metadata` for changes an app can't have; or when
	/// the database fails.
	pub async fn update_app(
		&self,
		me: &SignedIn,
		changes: AppChanges,
		info: &RequestInfo,
	) -> Result<ClientInfo> {
		self.portal()?;
		let client = self.owned(me, &changes.client_id).await?;
		let name = self.check_metadata(
			&changes.name,
			&changes.redirect_uris,
			&changes.post_logout_redirect_uris,
			&changes.scopes,
			&client.grant_types,
		)?;
		check_authentication(
			client.auth_method,
			&client.grant_types,
			changes.jwks.as_ref(),
		)?;
		let scopes = dedupe(changes.scopes);
		let removed: Vec<String> = client
			.scopes
			.iter()
			.filter(|scope| !scopes.contains(*scope))
			.cloned()
			.collect();

		let tx = self.inner.db.begin().await?;
		let result = async {
			Client::update()
				.filter(|row| row.id.eq(client.id))
				.set(|row| row.name, name.clone())
				.set(|row| row.redirect_uris, Json(dedupe(changes.redirect_uris)))
				.set(
					|row| row.post_logout_redirect_uris,
					Json(dedupe(changes.post_logout_redirect_uris)),
				)
				.set(|row| row.scopes, Json(scopes.clone()))
				.set(|row| row.jwks, changes.jwks.map(Json))
				.run(&tx)
				.await?;

			if !removed.is_empty() {
				narrow_grants(&tx, &client, &scopes, &removed).await?;
			}

			self.inner
				.auth
				.record(
					&tx,
					Some(me.account),
					EventKind::AppChanged,
					Some(&name),
					info,
				)
				.await?;
			Client::find(&tx, client.id)
				.await?
				.map(ClientInfo::from)
				.ok_or(IdpError::NotFound)
		}
		.await;

		finish(tx, result).await
	}

	/// Replaces the secret of an app `me` owns, and returns it, shown this
	/// once. The tokens the app got as itself end.
	///
	/// # Errors
	///
	/// [`IdpError::NotFound`] when the portal is off or `me` owns no such
	/// app; `invalid_client_metadata` for an app with no secret; or when the
	/// database fails.
	pub async fn rotate_app_secret(
		&self,
		me: &SignedIn,
		client_id: &str,
		info: &RequestInfo,
	) -> Result<RegisteredApp> {
		self.portal()?;
		let client = self.owned(me, client_id).await?;
		let tx = self.inner.db.begin().await?;
		let result = async {
			let secret = self
				.replace_secret(&tx, &client)
				.await?
				.ok_or_else(|| metadata("this app has no secret to replace"))?;
			self.inner
				.auth
				.record(
					&tx,
					Some(me.account),
					EventKind::AppChanged,
					Some(&client.name),
					info,
				)
				.await?;
			let app = Client::find(&tx, client.id)
				.await?
				.map(ClientInfo::from)
				.ok_or(IdpError::NotFound)?;
			Ok(RegisteredApp {
				app,
				secret: Some(secret.expose().to_owned()),
			})
		}
		.await;

		finish(tx, result).await
	}

	/// Deletes an app `me` owns, ending every grant and token it holds.
	/// Everyone who had connected it finds that in their audit trail.
	/// Returns whether there was one.
	///
	/// # Errors
	///
	/// [`IdpError::NotFound`] when the portal is off, or when the database
	/// fails.
	pub async fn delete_app(
		&self,
		me: &SignedIn,
		client_id: &str,
		info: &RequestInfo,
	) -> Result<bool> {
		self.portal()?;
		let Ok(client) = self.owned(me, client_id).await else {
			return Ok(false);
		};
		let tx = self.inner.db.begin().await?;
		let result = async {
			self.delete_client(&tx, &client, info).await?;
			self.inner
				.auth
				.record(
					&tx,
					Some(me.account),
					EventKind::AppDeleted,
					Some(&client.name),
					info,
				)
				.await?;
			Ok(true)
		}
		.await;

		finish(tx, result).await
	}

	/// Deletes an app, and tells everyone who had connected it.
	pub(crate) async fn delete_client(
		&self,
		tx: &Tx,
		client: &Client,
		info: &RequestInfo,
	) -> Result<()> {
		let grants = Grant::query()
			.filter(|grant| grant.client_id.eq(client.id))
			.all(tx)
			.await?;

		for grant in grants {
			self.inner
				.auth
				.record(
					tx,
					Some(grant.account_id),
					EventKind::AppDisconnected,
					Some(&client.name),
					info,
				)
				.await?;
		}

		Client::query()
			.filter(|row| row.id.eq(client.id))
			.delete(tx)
			.await?;
		Ok(())
	}

	/// The app `client_id`, if `me` owns it.
	async fn owned(&self, me: &SignedIn, client_id: &str) -> Result<Client> {
		self.client(client_id)
			.await?
			.filter(|client| client.owner_id == Some(me.account))
			.ok_or(IdpError::NotFound)
	}
}

/// Takes `removed` scopes from everyone who connected `client`: grants keep
/// what's left of `kept` (or go, with nothing left), and sign-ins carrying a
/// removed scope end.
pub(crate) async fn narrow_grants(
	tx: &impl Connection,
	client: &Client,
	kept: &[String],
	removed: &[String],
) -> Result<()> {
	let grants = Grant::query()
		.filter(|grant| grant.client_id.eq(client.id))
		.all(tx)
		.await?;

	for grant in grants {
		let left: Vec<String> = grant
			.scopes
			.iter()
			.filter(|scope| kept.contains(*scope))
			.cloned()
			.collect();

		if left.is_empty() {
			Grant::query()
				.filter(|row| row.id.eq(grant.id))
				.delete(tx)
				.await?;
		} else if left != grant.scopes.0 {
			Grant::update()
				.filter(|row| row.id.eq(grant.id))
				.set(|row| row.scopes, Json(left))
				.run(tx)
				.await?;
		}
	}

	let families = Family::query()
		.filter(|family| family.client_id.eq(client.id) & family.revoked_at.is_null())
		.all(tx)
		.await?;

	for family in families {
		if family.scopes.iter().any(|scope| removed.contains(scope)) {
			revoke_family(tx, family.id).await?;
		}
	}

	Ok(())
}
