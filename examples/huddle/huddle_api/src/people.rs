//! Who's here: the signed-in person, everyone they can add to a channel,
//! whether their session still holds, and an admin signing someone out on
//! every device.

use leptoskit_auth::Account;
use leptoskit_db::Id;
use leptoskit_rpc as rpc;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

/// Someone, as every screen shows them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Person {
	pub id: Id<Account>,
	pub name: String,
	pub is_admin: bool,
}

/// Whether the signed-in person's session still holds. Their open apps
/// watch it, and go back to the sign-in screen when it doesn't.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionState {
	/// Signed in, with a session nobody has ended.
	pub active: bool,
}

#[cfg(feature = "server")]
fn person(user: huddle_db::User) -> Person {
	Person {
		id: user.id,
		name: user.name,
		is_admin: user.is_admin,
	}
}

/// The signed-in person, or `None`.
#[rpc::query]
pub async fn whoami(cx: &rpc::QueryCx) -> rpc::Result<Option<Person>> {
	let Some(me) = cx.get::<leptoskit_auth::SignedIn>() else {
		return Ok(None);
	};
	Ok(huddle_db::User::find(cx.db(), me.account)
		.await?
		.map(person))
}

/// Everyone, by name: who can be added to a channel or tipped.
#[rpc::query]
pub async fn people(cx: &rpc::QueryCx) -> rpc::Result<Vec<Person>> {
	let users = huddle_db::User::query()
		.order_by(|user| user.name.asc())
		.all(cx.db())
		.await?;
	Ok(users.into_iter().map(person).collect())
}

/// Whether this session still holds.
///
/// The session table isn't live, so the query reads the person's own row,
/// which is: when an admin signs them out everywhere, that row changes, this
/// query runs again on every socket they have open, and now finds the
/// session ended.
#[rpc::query]
pub async fn my_session(cx: &rpc::QueryCx) -> rpc::Result<SessionState> {
	let Some(me) = cx.get::<leptoskit_auth::SignedIn>() else {
		return Ok(SessionState { active: false });
	};
	let Some(user) = huddle_db::User::find(cx.db(), me.account).await? else {
		return Ok(SessionState { active: false });
	};
	let session = me.session;
	let ended_by_admin = user
		.signed_out_at
		.is_some_and(|ended| ended >= me.authenticated_at);
	let row = leptoskit_auth::Session::query()
		.filter(|row| row.id.eq(session))
		.first(&cx.system("huddle: does this session still hold"))
		.await?;
	let revoked = row.is_none_or(|row| row.revoked_at.is_some());
	Ok(SessionState {
		active: !ended_by_admin && !revoked,
	})
}

/// Records that `user` was signed out everywhere, so their open apps hear
/// it. Run by [`sign_out_everywhere`] after the sessions end.
#[rpc::mutation]
pub async fn mark_signed_out(cx: &rpc::MutationCx, user: Id<Account>) -> rpc::Result<()> {
	// The users table lets only an admin change a row, so this fails for
	// anyone else.
	huddle_db::User::update()
		.filter(|row| row.id.eq(user))
		.set(|row| row.signed_out_at, Some(leptoskit_db::now()))
		.run(cx.tx())
		.await?;
	Ok(())
}

/// Ends every session `user` has, on every device, and drops their open
/// apps to the sign-in screen. Admins only.
#[rpc::action]
pub async fn sign_out_everywhere(cx: &rpc::ActionCx, user: Id<Account>) -> rpc::Result<()> {
	let me = cx.query(Whoami {}).await?;

	if !me.is_some_and(|me| me.is_admin) {
		return Err(rpc::Error::forbidden("only an admin can sign people out"));
	}

	crate::services::get()
		.auth
		.sign_out_everywhere(user, &leptoskit_auth::RequestInfo::default())
		.await
		.map_err(|error| rpc::Error::server(error.to_string()))?;
	cx.mutate(MarkSignedOut { user }).await
}
