//! The client side of `#[db::table]`. CI builds this crate for
//! `wasm32-unknown-unknown` without the `server` feature, and checks that no
//! database driver or schema code is in its dependency tree.

use leptoskit_db::Id;
use leptoskit_db::Timestamp;
use leptoskit_db::table;

/// Someone with an account.
#[table(name = "users", public)]
pub struct User {
	pub id: Id<Self>,
	#[db(max_len = 32)]
	pub handle: String,
	#[db(secret)]
	pub password_hash: String,
	pub theme: Theme,
	#[db(created_at)]
	pub created_at: Timestamp,
}

/// How the app looks.
#[derive(leptoskit_db::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme {
	Light,
	Dark,
}

/// On the client a user row can't exist: `User` has no values. Only the
/// browser build checks this: a native build of the whole workspace unifies
/// features, which turns `server` on here too.
#[cfg(target_arch = "wasm32")]
pub fn rows_are_unreachable(user: &User) -> ! {
	match *user {}
}

/// The public type is all a client has.
pub fn greeting(user: &PublicUser) -> String {
	format!("Hello, {} ({})", user.handle, user.theme)
}

/// Client-side validation, as a form would run it.
pub fn is_valid(user: &PublicUser) -> bool {
	user.validate().is_ok()
}

/// IDs still name their table.
pub fn same_user(left: Id<User>, right: Id<User>) -> bool {
	left == right
}
