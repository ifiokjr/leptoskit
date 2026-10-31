//! Projections per audience: `projection(SelfUser)` on a table, and
//! `#[db(private, in(SelfUser))]` on the private columns it shows.

#![cfg(feature = "server")]

use leptoskit_db::Email;
use leptoskit_db::Id;
use leptoskit_db::Timestamp;
use leptoskit_db::table;

/// A postal address, flattened into a table.
#[leptoskit_db::mixin]
#[derive(Clone)]
pub struct Address {
	pub city: String,
	#[db(private)]
	pub street: String,
}

/// Someone with an account. Everyone sees their handle; they see their own
/// email and address too; only the server sees the rest.
#[table(name = "members", public, projection(SelfMember, Staff))]
#[derive(Clone)]
pub struct Member {
	pub id: Id<Self>,
	#[db(max_len = 32)]
	pub handle: String,
	/// Where to write.
	#[db(private, in(SelfMember, Staff))]
	pub email: Email,
	#[db(private, in(Staff), max_len = 200)]
	pub notes: String,
	#[db(private, in(SelfMember))]
	pub phone: Option<String>,
	#[db(private)]
	pub risk: i32,
	#[db(secret)]
	pub password_hash: String,
	#[db(flatten, private, in(SelfMember))]
	pub home: Address,
	#[db(created_at)]
	pub joined_at: Timestamp,
}

fn member() -> Member {
	NewMember {
		handle: "ada".into(),
		email: Email::parse("ada@example.com").unwrap(),
		notes: "met at the conference".into(),
		phone: Some("+44".into()),
		risk: 3,
		password_hash: "hash".into(),
		home: Address {
			city: "London".into(),
			street: "1 Analytical Row".into(),
		},
	}
	.into_row()
}

#[test]
fn each_audience_sees_its_own_columns() {
	let row = member();

	let public = serde_json::to_value(PublicMember::from(row.clone())).unwrap();
	let mut fields: Vec<&str> = public
		.as_object()
		.unwrap()
		.keys()
		.map(String::as_str)
		.collect();
	fields.sort_unstable();
	assert_eq!(fields, ["handle", "id", "joined_at"]);

	let own = SelfMember::from(row.clone());
	assert_eq!(own.email.as_ref(), "ada@example.com");
	assert_eq!(own.phone.as_deref(), Some("+44"));
	// A flattened mixin arrives as its public type, as in `PublicMember`.
	assert_eq!(own.home.city, "London");
	let json = serde_json::to_value(&own).unwrap();
	let mut fields: Vec<&str> = json
		.as_object()
		.unwrap()
		.keys()
		.map(String::as_str)
		.collect();
	fields.sort_unstable();
	assert_eq!(
		fields,
		["email", "handle", "home", "id", "joined_at", "phone"]
	);
	assert_eq!(json["home"], serde_json::json!({ "city": "London" }));
	assert_eq!(serde_json::from_value::<SelfMember>(json).unwrap(), own);

	let staff = Staff::from(row);
	assert_eq!(staff.notes, "met at the conference");
	let json = serde_json::to_value(&staff).unwrap();
	assert!(json.get("phone").is_none() && json.get("risk").is_none());
	assert!(json.get("password_hash").is_none());
}

#[test]
fn a_projection_checks_the_rules_of_what_it_shows() {
	let mut own = SelfMember::from(member());
	assert!(own.validate().is_ok());
	own.handle = "x".repeat(33);
	let errors = own.validate().unwrap_err();
	assert_eq!(errors.field("handle").count(), 1, "{errors:?}");

	// A private column's rules hold wherever it's shown.
	let mut staff = Staff::from(member());
	staff.notes = "x".repeat(201);
	let errors = staff.validate().unwrap_err();
	assert_eq!(errors.field("notes").count(), 1, "{errors:?}");

	// Its schema carries them for generated clients.
	let schema = serde_json::to_value(schemars::schema_for!(SelfMember)).unwrap();
	assert_eq!(schema["properties"]["handle"]["maxLength"], 32);
	assert_eq!(
		schema["properties"]["email"]["description"],
		"Where to write."
	);
	let required: Vec<&str> = schema["required"]
		.as_array()
		.unwrap()
		.iter()
		.filter_map(|field| field.as_str())
		.collect();
	assert!(required.contains(&"email") && !required.contains(&"notes"));
}
