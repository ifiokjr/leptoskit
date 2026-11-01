//! The client side of `#[db::table]`. CI builds this crate for
//! `wasm32-unknown-unknown` without the `server` feature, and checks that no
//! database driver or schema code is in its dependency tree.

use leptoskit_db::Id;
use leptoskit_db::Scalar;
use leptoskit_db::table;

/// Someone with an account. They see their own email; everyone else sees
/// the public type.
#[table(name = "users", public, projection(SelfUser), extends = Owner)]
pub struct User {
	pub id: Id<Self>,
	#[db(max_len = 32)]
	pub handle: String,
	pub avatar: Option<leptoskit_db::Url>,
	#[db(private, in(SelfUser))]
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

/// One user following another. On the client a link is a marker, like a
/// table.
#[leptoskit_db::link(
	name = "follows",
	from = User as following,
	to = User as followers,
	from_column = "follower_id",
	to_column = "followed_id"
)]
pub struct Follow {}

/// A profile and its followers, as the server fetches it. The derive adds
/// nothing on the client, so the struct is an ordinary client type.
#[derive(leptoskit_db::Shape, Clone, Debug, serde::Serialize, serde::Deserialize)]
#[db(from = User, public)]
pub struct Profile {
	pub id: Id<User>,
	pub handle: String,
	#[db(via = followers)]
	pub followers: Vec<Follower>,
}

/// One follower, nested in a profile.
#[derive(leptoskit_db::Shape, Clone, Debug, serde::Serialize, serde::Deserialize)]
#[db(from = User, public)]
pub struct Follower {
	pub handle: String,
}

/// What a client does with a shape: reads it like any other value.
pub fn follower_count(profile: &Profile) -> usize {
	profile.followers.len()
}

/// The signed-in user's own view, with their email.
pub fn own_email(user: &SelfUser) -> &str {
	user.email.as_ref()
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

/// Anyone who can own something: a polymorphic table. On the client it is
/// a marker, like a table, and `PublicOwner` carries each kind's public
/// type.
#[leptoskit_db::polymorphic(name = "owners", public)]
pub enum Owner {
	User(User),
	Team(Team),
}

/// A team, one kind of owner.
#[table(name = "teams", public, extends = Owner)]
pub struct Team {
	pub id: Id<Self>,
	pub title: String,
}

/// What a client does with an owner: matches on its kind.
pub fn owner_name(owner: &PublicOwner) -> &str {
	match owner {
		PublicOwner::User(user) => &user.handle,
		PublicOwner::Team(team) => &team.title,
	}
}

/// An owner's kind is a client type too.
pub fn is_team(owner: &PublicOwner) -> bool {
	owner.kind() == OwnerKind::Team
}

/// IDs of the enum name it, as IDs of a table do.
pub fn same_owner(left: Id<Owner>, right: Id<Owner>) -> bool {
	left == right
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

/// An article with an embedding, which clients never receive, and a binary
/// code they do.
#[table(name = "articles", public)]
pub struct Article {
	pub id: Id<Self>,
	pub title: String,
	#[db(vector(distance = cosine, index = hnsw, source(title)))]
	pub embedding: Option<leptoskit_db::vector::Vector<3>>,
	#[db(public, vector(distance = hamming))]
	pub code: leptoskit_db::vector::BitVector<8>,
}

/// What a client does with a public article's code: compares it locally.
pub fn shared_bits(article: &PublicArticle, other: &leptoskit_db::vector::BitVector<8>) -> u32 {
	article
		.code
		.as_bytes()
		.iter()
		.zip(other.as_bytes())
		.map(|(left, right)| (left & right).count_ones())
		.sum()
}
