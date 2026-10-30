use leptoskit_db::Id;
use leptoskit_db::Shape;
use leptoskit_db::table;

#[table(name = "people")]
pub struct Person {
	pub id: Id<Self>,
	pub name: String,
	#[db(private)]
	pub email: String,
	#[db(backlink = friends)]
	pub friend_id: Option<Id<Person>>,
}

/// A public shape may only read public columns.
#[derive(Shape)]
#[db(from = Person, public)]
pub struct Card {
	pub name: String,
	pub email: String,
}

/// Nor may it nest a shape that isn't public.
#[derive(Shape)]
#[db(from = Person, public)]
pub struct Friends {
	#[db(via = friends)]
	pub friends: Vec<Contact>,
}

#[derive(Shape)]
#[db(from = Person)]
pub struct Contact {
	pub email: String,
}

/// A shape follows only links the table has.
#[derive(Shape)]
#[db(from = Person)]
pub struct Lost {
	#[db(count(enemies))]
	pub enemies: i64,
}

fn main() {}
