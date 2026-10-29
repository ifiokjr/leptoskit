//! The client side of `#[db::table]`. CI builds this crate for
//! `wasm32-unknown-unknown` without the `server` feature, and checks that no
//! database driver or schema code is in its dependency tree.

use leptoskit_db::Id;
use leptoskit_db::Scalar;
use leptoskit_db::table;

/// Someone with an account.
#[table(name = "users", public)]
pub struct User {
	pub id: Id<Self>,
	#[db(max_len = 32)]
	pub handle: String,
	pub avatar: Option<leptoskit_db::Url>,
	#[db(private)]
	pub email: leptoskit_db::Email,
	#[db(secret)]
	pub password_hash: String,
	pub badge: Badge,
	pub theme: Theme,
	#[db(flatten)]
	pub times: leptoskit_db::Timestamps,
	#[db(flatten, prefix = "home_")]
	pub home: Place,
}

/// A label shown next to a name. Scalars are client types too: a form can
/// parse one before it sends anything.
#[derive(Scalar, Clone, Debug, PartialEq, Eq)]
#[db(max_len = 12, normalize = uppercase)]
pub struct Badge(String);

/// Parses a badge typed into a form, as the server would.
pub fn badge(input: &str) -> Option<Badge> {
	Badge::parse(input).ok()
}

/// Somewhere on a map. Mixins are client types too: the public row nests
/// `PublicPlace`.
#[leptoskit_db::mixin]
pub struct Place {
	#[db(max_len = 80)]
	pub name: String,
	#[db(private)]
	pub directions: Option<String>,
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

/// A flattened mixin arrives as its own public type.
pub fn hometown(user: &PublicUser) -> &str {
	&user.home.name
}

/// Client-side validation, as a form would run it.
pub fn is_valid(user: &PublicUser) -> bool {
	user.validate().is_ok()
}

/// IDs still name their table.
pub fn same_user(left: Id<User>, right: Id<User>) -> bool {
	left == right
}
