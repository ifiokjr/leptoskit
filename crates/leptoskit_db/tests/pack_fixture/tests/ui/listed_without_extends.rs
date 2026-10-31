use leptoskit_db::Id;

/// Anyone who can own something.
#[leptoskit_db::polymorphic(name = "owners")]
pub enum Owner {
	User(User),
	Team(Team),
}

/// A user, which says it's an owner.
#[leptoskit_db::table(name = "users", extends = Owner)]
pub struct User {
	pub id: Id<Self>,
}

/// A team the enum lists, which doesn't say it's an owner: its key would
/// point at no supertype row.
#[leptoskit_db::table(name = "teams")]
pub struct Team {
	pub id: Id<Self>,
}

fn main() {}
