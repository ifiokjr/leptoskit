//! Custom scalars as columns: their rules become database checks, they round
//! trip through both databases, and a value that breaks them can't be read
//! back as the scalar.

#![cfg(all(feature = "postgres", feature = "sqlite"))]

mod support;

use leptoskit_db::Db;
use leptoskit_db::Dialect;
use leptoskit_db::Email;
use leptoskit_db::Id;
use leptoskit_db::Scalar;
use leptoskit_db::Table;
use leptoskit_db::Url;
use leptoskit_db::table;
use leptoskit_db_schema::CheckRule;
use leptoskit_db_schema::ColumnType;
use leptoskit_db_schema::RequireHints;
use leptoskit_db_schema::Schema;

/// A public handle.
#[derive(Scalar, Clone, Debug, PartialEq, Eq)]
#[db(min_len = 3, max_len = 20, regex = "^[a-z0-9_]+$", normalize = lowercase)]
pub struct Handle(String);

/// A rating out of five.
#[derive(Scalar, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[db(min = 1, max = 5)]
pub struct Stars(i16);

/// Someone's profile.
#[table(name = "profiles", public)]
pub struct Profile {
	pub id: Id<Self>,
	#[db(unique)]
	pub handle: Handle,
	#[db(private)]
	pub email: Option<Email>,
	pub site: Option<Url>,
	pub stars: Stars,
}

fn new_profile(handle: &str, stars: i16) -> NewProfile {
	NewProfile {
		handle: Handle::parse(handle).unwrap(),
		email: Some(Email::parse(format!("{handle}@example.com")).unwrap()),
		site: None,
		stars: Stars::parse(stars).unwrap(),
	}
}

#[test]
fn scalar_rules_become_checks_on_every_column_of_their_type() {
	let table = Profile::definition();
	let column = |name: &str| {
		table
			.columns
			.iter()
			.find(|column| column.name == name)
			.unwrap()
	};
	let rules: Vec<&CheckRule> = table.checks.iter().map(|check| &check.rule).collect();

	assert_eq!(column("handle").ty, ColumnType::TEXT);
	assert_eq!(column("stars").ty, ColumnType::Int16);
	// A maximum alone narrows the column instead of adding a check.
	assert_eq!(column("email").ty, ColumnType::Text { max_len: Some(254) });
	assert!(column("email").nullable);

	assert!(rules.contains(&&CheckRule::Length {
		column: "handle".into(),
		min: Some(3),
		max: Some(20),
	}));
	assert!(rules.contains(&&CheckRule::Regex {
		column: "handle".into(),
		pattern: "^[a-z0-9_]+$".into(),
	}));
	assert!(rules.contains(&&CheckRule::Range {
		column: "stars".into(),
		min: Some(1.0),
		max: Some(5.0),
	}));
	assert!(
		rules
			.iter()
			.any(|rule| matches!(rule, CheckRule::Regex { column, .. } if column == "email"))
	);
}

#[test]
fn scalars_check_numbers_and_describe_them() {
	assert_eq!(*Stars::parse(4).unwrap(), 4);
	let errors = Stars::parse(9).unwrap_err();
	assert_eq!(errors.to_string(), "`Stars` can't be more than 5");
	assert!(Stars::try_from(0).is_err());

	let schema = serde_json::to_value(schemars::schema_for!(Stars)).unwrap();
	assert_eq!(
		(schema["minimum"].clone(), schema["maximum"].clone()),
		(1.into(), 5.into())
	);

	// The public type carries the scalar, so clients get the same checks.
	let public: PublicProfile = serde_json::from_str(
		r#"{"id":"0192f0c3-7a5e-7d4e-8b8e-1f6f2b1c0d9a","handle":"ADA","site":null,"stars":3}"#,
	)
	.unwrap();
	assert_eq!(public.handle.as_inner(), "ada");
	assert!(
		serde_json::from_str::<PublicProfile>(
			r#"{"id":"0192f0c3-7a5e-7d4e-8b8e-1f6f2b1c0d9a","handle":"ada","site":null,"stars":6}"#
		)
		.is_err()
	);
}

async fn create_schema(db: &Db) {
	let dialect = db.dialect();
	let schema = leptoskit_db::schema(dialect).unwrap();
	let empty = Schema::empty(dialect);
	let plan = leptoskit_db_schema::diff(&empty, &schema, &mut RequireHints).unwrap();
	let migration = leptoskit_db_schema::render(&plan, &empty, &schema);
	db.run_script(&migration.statements.join(";\n"))
		.await
		.unwrap();
}

async fn on_both<F, Fut>(check: F)
where
	F: Fn(Db) -> Fut,
	Fut: Future<Output = ()>,
{
	let db = Db::connect("sqlite::memory:").await.unwrap();
	create_schema(&db).await;
	check(db).await;

	if let Some(cluster) = support::postgres::PostgresCluster::start() {
		let db = Db::connect(&cluster.database()).await.unwrap();
		create_schema(&db).await;
		check(db).await;
	}
}

#[tokio::test(flavor = "multi_thread")]
async fn scalar_columns_round_trip_and_filter() {
	on_both(async |db| {
		let mut ada = new_profile("Ada", 5).insert(&db).await.unwrap();
		new_profile("grace", 3).insert(&db).await.unwrap();

		let found = Profile::query()
			.filter(|profile| profile.handle.eq(Handle::parse("ADA").unwrap()))
			.one(&db)
			.await
			.unwrap();
		assert_eq!(found.handle.as_inner(), "ada");
		assert_eq!(found.email.as_ref().unwrap().as_inner(), "ada@example.com");
		assert_eq!(found.stars, Stars::parse(5).unwrap());

		let rated = Profile::query()
			.filter(|profile| {
				profile.stars.gte(Stars::parse(4).unwrap()) & profile.handle.contains("d")
			})
			.count(&db)
			.await
			.unwrap();
		assert_eq!(rated, 1);

		ada.site = Some(Url::parse("https://ada.example").unwrap());
		ada.save(&db).await.unwrap();
		let site = Profile::get(&db, ada.id).await.unwrap().site.unwrap();
		assert_eq!(site.as_inner(), "https://ada.example");

		let taken = new_profile("ada", 1).insert(&db).await.unwrap_err();
		assert!(taken.is_conflict(), "{taken:?}");
	})
	.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_database_enforces_scalar_rules_too() {
	on_both(async |db| {
		let id = Id::<Profile>::new();
		let key = match db.dialect() {
			Dialect::Postgres => format!("'{}'", id.uuid()),
			Dialect::Sqlite => format!("X'{}'", id.uuid().simple()),
		};
		let insert = |handle: &str, stars: i16| {
			format!("INSERT INTO profiles (id, handle, stars) VALUES ({key}, '{handle}', {stars})")
		};

		// Out of range: both databases check numbers and lengths.
		assert!(db.run_script(&insert("valid", 9)).await.is_err());
		assert!(db.run_script(&insert("ab", 3)).await.is_err());

		// Postgres checks the pattern too; SQLite leaves it to the app, so
		// reading the row back is where the scalar refuses it.
		let outcome = db.run_script(&insert("Not Valid", 3)).await;

		match db.dialect() {
			Dialect::Postgres => assert!(outcome.is_err()),
			Dialect::Sqlite => {
				outcome.unwrap();
				let error = Profile::get(&db, id).await.unwrap_err().to_string();
				assert!(error.contains("stored value"), "{error}");
			}
		}
	})
	.await;
}
