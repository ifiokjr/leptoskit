//! Who a request is from, as access policies see it.

#[cfg(feature = "server")]
use leptoskit_db::Context;
#[cfg(feature = "server")]
use leptoskit_db::Expr;
use leptoskit_db::Id;
use leptoskit_db::Timestamp;

use crate::Aal;
use crate::Account;
use crate::Session;

/// The signed-in person behind a request, from a session that is live right
/// now. The auth layer adds it to the request's [`Context`], so policies
/// can read it:
///
/// ```ignore
/// #[db::table(name = "posts")]
/// #[db::policy(all, |post, cx| cx.signed_in(|me| post.author_id.eq(me.user_id())))]
/// pub struct Post { /* … */ }
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignedIn {
	/// Their account.
	pub account: Id<Account>,
	/// The session the request came with.
	pub session: Id<Session>,
	/// How strongly they proved who they are.
	pub aal: Aal,
	/// When they last proved it.
	pub authenticated_at: Timestamp,
	/// When they last completed a second factor.
	pub mfa_at: Option<Timestamp>,
}

impl SignedIn {
	/// Their ID in the app's own user table, which shares its key with
	/// [`Account`]: `Id<User>`, `Id<Author>`, or whatever the table is.
	pub const fn user_id<U>(&self) -> Id<U> {
		Id::from_uuid(self.account.uuid())
	}
}

/// The signed-in account, as a setting: policies compare with it through
/// `cx.setting::<SignedIn>()`, which Postgres can read too, so the auth
/// tables' policies also run as row-level security.
#[cfg(feature = "server")]
impl leptoskit_db::Setting for SignedIn {
	type Value = Id<Account>;

	const NAME: &'static str = "account";

	fn value(&self) -> Id<Account> {
		self.account
	}
}

#[cfg(feature = "server")]
leptoskit_db::register_setting!(SignedIn);

#[cfg(feature = "server")]
/// Conditions on the signed-in person, for policies.
pub trait AuthContext {
	/// `condition`, built from the signed-in person, or [`Expr::never`] for a
	/// request without one, so a rule about the signed-in user matches
	/// nothing for everyone else.
	fn signed_in(&self, condition: impl FnOnce(&SignedIn) -> Expr) -> Expr;

	/// The signed-in person's ID in the app's user table, if there is one.
	fn user_id<U>(&self) -> Option<Id<U>>;
}

#[cfg(feature = "server")]
impl AuthContext for Context {
	fn signed_in(&self, condition: impl FnOnce(&SignedIn) -> Expr) -> Expr {
		self.global(condition)
	}

	fn user_id<U>(&self) -> Option<Id<U>> {
		self.get::<SignedIn>().map(SignedIn::user_id)
	}
}

/// The studio's `account` persona: a request from a signed-in account, by
/// its ID, as [`crate::context`] builds one. It signed in with one factor
/// and has no session of its own, so a rule that asks for a second factor
/// or a particular session refuses it.
#[cfg(any(feature = "postgres", feature = "sqlite"))]
fn studio_account(id: &str) -> Result<Context, String> {
	let account = id
		.parse::<Id<Account>>()
		.map_err(|error| format!("`{id}` isn't an account ID: {error}"))?;
	Ok(crate::context(Some(SignedIn {
		account,
		session: Id::parse("00000000-0000-0000-0000-000000000000")
			.map_err(|error| error.to_string())?,
		aal: Aal::One,
		authenticated_at: leptoskit_db::now(),
		mfa_at: None,
	})))
}

#[cfg(any(feature = "postgres", feature = "sqlite"))]
leptoskit_db::register_persona!(leptoskit_db::Persona {
	name: "account",
	describe: "a signed-in account, by its ID",
	context: studio_account,
});

#[cfg(all(test, any(feature = "postgres", feature = "sqlite")))]
mod tests {
	use super::*;

	#[test]
	fn the_studio_views_the_data_as_a_signed_in_account() {
		let account = Id::<Account>::new();
		let cx = studio_account(&account.to_string()).unwrap();
		let signed_in = cx.get::<SignedIn>().unwrap();
		assert_eq!(signed_in.account, account);
		assert_eq!(signed_in.aal, Aal::One);
		assert_eq!(cx.actor(), Some(leptoskit_db::Actor::of(account)));
		assert_eq!(cx.user_id::<Account>(), Some(account));

		let error = studio_account("ada").unwrap_err();
		assert!(error.contains("`ada` isn't an account ID"), "{error}");
	}
}
