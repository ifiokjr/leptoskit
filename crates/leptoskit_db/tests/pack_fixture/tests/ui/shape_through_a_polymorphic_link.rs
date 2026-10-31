use leptoskit_db::Id;

/// Anyone who can own something.
#[leptoskit_db::polymorphic(name = "owners")]
pub enum Owner {
	User(User),
}

/// A user.
#[leptoskit_db::table(name = "users", extends = Owner)]
pub struct User {
	pub id: Id<Self>,
	pub handle: String,
}

/// A budget, owned by any owner.
#[leptoskit_db::table(name = "budgets")]
pub struct Budget {
	pub id: Id<Self>,
	pub owner_id: Id<Owner>,
}

/// A shape can't select a polymorphic link's rows: they are of several
/// tables. Load them with `budget.owner(&db)` instead.
#[derive(leptoskit_db::Shape)]
#[db(from = Budget)]
pub struct BudgetCard {
	pub id: Id<Budget>,
	#[db(via = owner)]
	pub owner: OwnerCard,
}

#[derive(leptoskit_db::Shape)]
#[db(from = Owner)]
pub struct OwnerCard {
	pub id: Id<Owner>,
}

fn main() {}
