use leptoskit_db::Id;

/// Anyone who can own something.
#[leptoskit_db::polymorphic(name = "owners")]
pub enum Owner {
	User(User),
}

/// A user, listed.
#[leptoskit_db::table(name = "users", extends = Owner)]
pub struct User {
	pub id: Id<Self>,
}

/// A team that says it's an owner, which the enum doesn't list: no match
/// on the enum could handle it.
#[leptoskit_db::table(name = "teams", extends = Owner)]
pub struct Team {
	pub id: Id<Self>,
}

fn main() {}
