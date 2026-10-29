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
/// pub struct Post { … }
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
