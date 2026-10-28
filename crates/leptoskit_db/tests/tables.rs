//! Tables declared with `#[db::table]`, as the server sees them: their schema,
//! their rows, and the public projection.

#![cfg(feature = "server")]

use leptoskit_db::Column;
use leptoskit_db::Id;
use leptoskit_db::Json;
use leptoskit_db::Table;
use leptoskit_db::Timestamp;
use leptoskit_db::Value;
use leptoskit_db::table;
use leptoskit_db_schema::Dialect;
use leptoskit_db_schema::RequireHints;
use leptoskit_db_schema::Schema;

/// What someone may do.
#[derive(leptoskit_db::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
	Member,
	#[db(rename = "mod")]
	Moderator,
	SuperAdmin,
}

/// Someone with an account.
#[table(name = "users", public)]
#[derive(Clone)]
pub struct User {
	pub id: Id<Self>,
	/// Shown on profiles.
	#[db(unique(except = deleted_at), min_len = 3, max_len = 32, regex = "^[a-z0-9_]+$")]
	pub handle: String,
	#[db(private)]
	pub email: String,
	#[db(secret)]
	pub password_hash: String,
	pub role: Role,
	#[db(min = 0, max = 100, default = 0)]
	pub karma: i32,
	pub settings: Json<Vec<String>>,
	#[db(created_at)]
	pub created_at: Timestamp,
	#[db(updated_at)]
	pub updated_at: Timestamp,
	#[db(deleted_at)]
	pub deleted_at: Option<Timestamp>,
}

/// Something a user wrote.
#[table(name = "posts", public, index(author_id, published_at))]
pub struct Post {
	pub id: Id<Post>,
	#[db(on_delete = cascade)]
	pub author_id: Id<User>,
	pub editor_id: Option<Id<User>>,
	#[db(min_len = 1, max_len = 200)]
	pub title: String,
	#[db(one_of("draft", "published"), default = "draft")]
	pub status: String,
	pub published_at: Option<Timestamp>,
}

/// A tag on a post, private to the server.
#[table(
	name = "post_tags",
	namespace = "content",
	unique(post_id, position),
	check(name = "content_post_tags_position_check", sql = "position >= 0")
)]
pub struct PostTag {
	#[db(key)]
	pub post_id: Id<Post>,
	#[db(key)]
	pub tag: String,
	pub position: i16,
}

fn new_user() -> NewUser {
	NewUser {
		handle: "ada".into(),
		email: "ada@example.com".into(),
		password_hash: "hash".into(),
		role: Role::Moderator,
		karma: 10,
		settings: Json(vec!["dark".into()]),
	}
}

#[test]
fn every_table_exports_into_one_schema() {
	let schema = leptoskit_db::schema(Dialect::Postgres).unwrap();
	insta::assert_snapshot!(schema.to_json());
}

#[test]
fn the_exported_schema_renders_for_both_dialects() {
	for dialect in [Dialect::Postgres, Dialect::Sqlite] {
		let empty = Schema::empty(dialect);
		let schema = leptoskit_db::schema(dialect)
			.unwrap()
			.without_rename_hints();
		let plan = leptoskit_db_schema::diff(&empty, &schema, &mut RequireHints).unwrap();
		let migration = leptoskit_db_schema::render(&plan, &empty, &schema);
		insta::assert_snapshot!(format!("create__{dialect}"), migration.sql());
	}
}

#[test]
fn an_insert_fills_in_the_key_and_timestamps() {
	let before = leptoskit_db::now();
	let user = new_user().into_row();

	assert!(user.created_at >= before && user.created_at == user.updated_at);
	assert_eq!(user.deleted_at, None);
	assert_ne!(user.id, new_user().into_row().id);
	assert_eq!(
		user.created_at.timestamp_subsec_nanos() % 1_000_000,
		0,
		"milliseconds only"
	);
}

#[test]
fn rows_round_trip_through_values() {
	let user = new_user().into_row();
	let values = user.to_values().unwrap();
	assert_eq!(values.len(), User::COLUMNS.len());

	let decoded = User::from_values(values).unwrap();
	assert_eq!(format!("{decoded:?}"), format!("{user:?}"));
}

#[test]
fn decoding_reports_the_column() {
	let mut values = new_user().into_row().to_values().unwrap();
	values[1] = Value::Int(3);
	assert_eq!(
		User::from_values(values).unwrap_err().to_string(),
		"users.handle: expected text, found integer"
	);

	let error = User::from_values(Vec::new()).unwrap_err();
	assert_eq!(error.to_string(), "users.id: the row has too few columns");
}

#[test]
fn debug_redacts_secret_columns() {
	let debug = format!("{:?}", new_user().into_row());
	assert!(debug.contains("password_hash: <secret>") && !debug.contains("hash\""));
	assert!(
		debug.contains("email: \"ada@example.com\""),
		"private isn't secret: {debug}"
	);
}

#[test]
fn the_public_type_leaves_out_private_and_secret_columns() {
	let mut user = new_user().into_row();
	user.id = Id::parse("0192d3f4-0000-7000-8000-000000000001").unwrap();
	user.created_at = "2026-10-28T10:00:00.123Z".parse().unwrap();
	user.updated_at = user.created_at;

	let public = PublicUser::from(user.clone());
	insta::assert_json_snapshot!(public);

	let json = serde_json::to_string(&public).unwrap();
	assert!(!json.contains("email") && !json.contains("password"));
	assert_eq!(serde_json::from_str::<PublicUser>(&json).unwrap(), public);
}

#[test]
fn the_public_type_describes_its_rules_to_generated_clients() {
	insta::assert_json_snapshot!(schemars::schema_for!(PublicPost));
}

#[test]
fn validation_checks_every_rule_a_column_declares() {
	let mut user = new_user();
	user.handle = "Al".into();
	user.karma = 101;
	let errors = user.validate().unwrap_err();
	insta::assert_snapshot!(errors.to_string());
	assert_eq!(errors.field("handle").count(), 2);

	let post = NewPost {
		author_id: Id::new(),
		editor_id: None,
		title: String::new(),
		status: "archived".into(),
		published_at: None,
	};
	insta::assert_snapshot!(post.validate().unwrap_err().to_string());
	assert!(new_user().validate().is_ok());
}

#[test]
fn public_types_validate_too() {
	let public = PublicPost {
		id: Id::new(),
		author_id: Id::new(),
		editor_id: None,
		title: "x".repeat(201),
		status: "draft".into(),
		published_at: None,
	};
	assert_eq!(
		public.validate().unwrap_err().to_string(),
		"`title` can have at most 200 characters"
	);
}

#[test]
fn enums_store_their_values_as_text() {
	assert_eq!(Role::VALUES, ["member", "mod", "super_admin"]);
	assert_eq!(Role::SuperAdmin.to_string(), "super_admin");
	assert_eq!("mod".parse::<Role>(), Ok(Role::Moderator));
	assert_eq!(
		"admin".parse::<Role>().unwrap_err().to_string(),
		"`admin` isn't a Role; expected `member`, `mod`, `super_admin`"
	);
	assert_eq!(serde_json::to_string(&Role::Member).unwrap(), "\"member\"");
	assert_eq!(
		serde_json::from_str::<Role>("\"mod\"").unwrap(),
		Role::Moderator
	);
	assert!(serde_json::from_str::<Role>("\"nope\"").is_err());
	assert_eq!(
		Role::from_value(Role::Moderator.to_value().unwrap()).unwrap(),
		Role::Moderator
	);
	assert!(Role::from_value(Value::Int(1)).is_err());
	insta::assert_json_snapshot!(schemars::schema_for!(Role));
}

#[test]
fn ids_are_typed_uuids() {
	let id: Id<User> = Id::new();
	let text = id.to_string();
	assert_eq!(text.parse::<Id<User>>().unwrap(), id);
	assert_eq!(serde_json::to_string(&id).unwrap(), format!("\"{text}\""));
	assert_eq!(
		serde_json::from_str::<Id<User>>(&format!("\"{text}\"")).unwrap(),
		id
	);
	assert_eq!(format!("{id:?}"), format!("Id({text})"));
	assert_eq!(uuid::Uuid::from(id), id.uuid());
	assert!(Id::<User>::parse("nope").is_err());

	let later: Id<User> = Id::new();
	assert!(later > id, "UUIDv7 orders by creation");
	assert_eq!(Id::<User>::default().uuid().get_version_num(), 7);

	let mut seen = std::collections::HashSet::new();
	seen.insert(id);
	assert!(seen.contains(&id));
}

#[test]
fn natural_keys_give_stable_ids_per_table() {
	let one = Id::<User>::from_natural_key(&["solana", "ab"]);
	assert_eq!(one, Id::<User>::from_natural_key(&["solana", "ab"]));
	assert_ne!(one, Id::<User>::from_natural_key(&["solanaa", "b"]));
	assert_ne!(
		one.uuid(),
		Id::<Post>::from_natural_key(&["solana", "ab"]).uuid()
	);
}

#[test]
fn primitives_decode_from_either_driver() {
	assert!(bool::from_value(Value::Int(1)).unwrap());
	assert!(bool::from_value(Value::Text("t".into())).is_err());
	assert_eq!(i16::from_value(Value::Int(7)).unwrap(), 7);
	assert!(
		i16::from_value(Value::Int(70_000))
			.unwrap_err()
			.to_string()
			.contains("doesn't fit i16")
	);
	assert!(i64::from_value(Value::Float(1.0)).is_err());
	assert!(u64::MAX.to_value().is_err());
	assert_eq!(
		f32::from_value(Value::Float(1.5)).unwrap().to_bits(),
		1.5_f32.to_bits()
	);
	assert_eq!(
		f64::from_value(Value::Int(2)).unwrap().to_bits(),
		2.0_f64.to_bits()
	);
	assert!(f64::from_value(Value::Bool(true)).is_err());
	assert!(String::from_value(Value::Bytes(vec![1])).is_err());
	assert_eq!(Vec::<u8>::from_value(Value::Bytes(vec![1])).unwrap(), [1]);
	assert!(Vec::<u8>::from_value(Value::Null).is_err());
}

#[test]
fn uuids_and_times_decode_from_either_driver() {
	let uuid = uuid::Uuid::now_v7();
	assert_eq!(
		uuid::Uuid::from_value(Value::Bytes(uuid.as_bytes().to_vec())).unwrap(),
		uuid
	);
	assert!(uuid::Uuid::from_value(Value::Bytes(vec![1, 2])).is_err());
	assert!(uuid::Uuid::from_value(Value::Int(1)).is_err());

	let at: Timestamp = "2026-10-28T10:00:00.123Z".parse().unwrap();
	assert_eq!(
		Timestamp::from_value(Value::Text("2026-10-28T10:00:00.123Z".into())).unwrap(),
		at
	);
	assert!(Timestamp::from_value(Value::Text("yesterday".into())).is_err());
	assert!(Timestamp::from_value(Value::Int(0)).is_err());
	assert_eq!(Timestamp::from_value(at.to_value().unwrap()).unwrap(), at);

	let date = chrono::NaiveDate::from_ymd_opt(2026, 10, 28).unwrap();
	assert_eq!(
		chrono::NaiveDate::from_value(Value::Text("2026-10-28".into())).unwrap(),
		date
	);
	assert_eq!(
		chrono::NaiveDate::from_value(date.to_value().unwrap()).unwrap(),
		date
	);
	assert!(chrono::NaiveDate::from_value(Value::Int(0)).is_err());

	assert_eq!(
		Json::<Vec<u8>>::from_value(Value::Text("[1,2]".into())).unwrap(),
		Json(vec![1, 2])
	);
	assert!(Json::<Vec<u8>>::from_value(Value::Text("{".into())).is_err());
	assert!(Json::<Vec<u8>>::from_value(Value::Int(1)).is_err());
	assert_eq!(*Json(vec![1]), vec![1]);

	assert_eq!(Option::<i32>::from_value(Value::Null).unwrap(), None);
	assert_eq!(Option::<i32>::None.to_value().unwrap(), Value::Null);
	assert!(Option::<Role>::allowed_values().is_some());
	assert!(<Option<Id<User>> as Column>::references().is_some());
}

#[test]
fn every_value_kind_has_a_name() {
	let kinds: Vec<&str> = [
		Value::Null,
		Value::Bool(true),
		Value::Int(1),
		Value::Float(1.0),
		Value::Text(String::new()),
		Value::Bytes(Vec::new()),
		Value::Uuid(uuid::Uuid::nil()),
		Value::Timestamp(leptoskit_db::now()),
		Value::Date(chrono::NaiveDate::MIN),
		Value::Json(serde_json::Value::Null),
	]
	.iter()
	.map(Value::kind)
	.collect();
	assert_eq!(
		kinds,
		[
			"null",
			"bool",
			"integer",
			"float",
			"text",
			"bytes",
			"uuid",
			"timestamp",
			"date",
			"json"
		]
	);
	assert_eq!(leptoskit_db::EncodeError::new("nope").to_string(), "nope");
}

#[test]
fn tables_know_their_names_and_markers() {
	assert_eq!(PostTag::table_name().to_string(), "content.post_tags");
	assert_eq!(PostTag::flat_name(), "content_post_tags");
	assert_eq!(PostTag::KEY, ["post_id", "tag"]);
	assert_eq!(Post::CREATED_AT, None);
	assert_eq!(User::DELETED_AT, Some("deleted_at"));

	let mut user = new_user().into_row();
	user.mark_deleted();
	assert!(user.deleted_at.is_some());

	let before = user.updated_at;
	std::thread::sleep(std::time::Duration::from_millis(2));
	user.touch();
	assert!(user.updated_at > before);

	let mut post = NewPost {
		author_id: user.id,
		editor_id: None,
		title: "t".into(),
		status: "draft".into(),
		published_at: None,
	}
	.into_row();
	post.touch();
	post.mark_deleted();
}

/// A table that was renamed, with a JSON column of a custom type.
#[table(name = "accounts", renamed_from = "members")]
pub struct Account {
	pub id: Id<Self>,
	pub nickname: Option<String>,
	pub small: u8,
	pub medium: u16,
	pub large: u32,
	pub huge: u64,
	pub ratio: f32,
	pub avatar: Vec<u8>,
	pub born_on: chrono::NaiveDate,
	pub external: uuid::Uuid,
	pub flags: Json<std::collections::BTreeMap<String, bool>>,
	pub raw: Option<Json<serde_json::Value>>,
}

#[test]
fn every_column_type_maps_and_round_trips() {
	let definition = Account::definition();
	assert_eq!(definition.renamed_from.unwrap().name, "members");

	let types: Vec<String> = definition
		.columns
		.iter()
		.map(|column| format!("{} {}", column.name, column.ty))
		.collect();
	insta::assert_snapshot!(types.join("\n"));

	let mut flags = std::collections::BTreeMap::new();
	flags.insert("beta".to_string(), true);
	let account = NewAccount {
		nickname: Some("ada".into()),
		small: 1,
		medium: 2,
		large: 3,
		huge: 4,
		ratio: 0.5,
		avatar: vec![1, 2, 3],
		born_on: chrono::NaiveDate::from_ymd_opt(1815, 12, 10).unwrap(),
		external: uuid::Uuid::nil(),
		flags: Json(flags),
		raw: Some(Json(serde_json::json!({ "a": 1 }))),
	}
	.into_row();
	let decoded = Account::from_values(account.to_values().unwrap()).unwrap();
	assert_eq!(format!("{decoded:?}"), format!("{account:?}"));
	assert!(account.flags.contains_key("beta"));

	let mut edited = decoded;
	edited.flags.insert("dark".into(), false);
	assert_eq!(edited.flags.len(), 2);
}

#[test]
fn rules_check_optional_and_borrowed_values() {
	use leptoskit_db::__private::rules;

	let mut errors = leptoskit_db::ValidationErrors::default();
	rules::length(
		&mut errors,
		"bio",
		&Some("toolong".to_string()),
		None,
		Some(3),
	);
	rules::length(&mut errors, "bio", &None::<String>, Some(1), None);
	rules::length(&mut errors, "word", "a", Some(2), None);
	rules::range(&mut errors, "age", &Some(3_u8), Some(5.0), None);
	rules::range(&mut errors, "age", &None::<i64>, Some(5.0), None);
	rules::range(&mut errors, "big", &u64::MAX, None, Some(1.0));
	rules::range(&mut errors, "small", &-1_i64, Some(0.0), None);
	rules::one_of(&mut errors, "size", &Some("xl".to_string()), &["s", "m"]);
	rules::pattern(&mut errors, "code", &Some("ab".to_string()), "^[0-9]+$");
	rules::pattern(&mut errors, "code", &None::<String>, "^[0-9]+$");

	insta::assert_snapshot!(errors.to_string());
	assert_eq!(errors.field("age").count(), 1);
	assert!(!errors.is_empty());
	insta::assert_json_snapshot!(errors);
}
