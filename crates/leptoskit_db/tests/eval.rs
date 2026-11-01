//! `Expr::eval` against both databases: property tests build random
//! conditions of every kind `Expr` has and random rows, run each condition
//! as a filter on the stored row, and check that `eval` is sound. It may
//! say false only where SQL does, and whenever it gives an answer, that
//! answer is SQL's. A deliberately broken `eval` shows the check catches
//! an unsound answer.

#![cfg(all(feature = "postgres", feature = "sqlite"))]

mod support;

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use chrono::NaiveDate;
use chrono::TimeDelta;
use chrono::TimeZone;
use chrono::Utc;
use leptoskit_db::Comparison;
use leptoskit_db::Context;
use leptoskit_db::Db;
use leptoskit_db::Expr;
use leptoskit_db::Id;
use leptoskit_db::Json;
use leptoskit_db::Setting;
use leptoskit_db::Table;
use leptoskit_db::Timestamp;
use leptoskit_db::Tri;
use leptoskit_db::Value;
use leptoskit_db::geo::Geo;
use leptoskit_db::geo::Point;
use leptoskit_db::geo::Rect;
use leptoskit_db::geo::Srid;
use leptoskit_db::geo::coord;
use leptoskit_db::geo::point;
use leptoskit_db::table;
use leptoskit_db_schema::RequireHints;
use leptoskit_db_schema::Schema;
use leptoskit_db_schema::TableName;
use proptest::prelude::*;
use proptest::test_runner::Config;
use proptest::test_runner::TestCaseError;
use proptest::test_runner::TestRunner;
use uuid::Uuid;

/// A row with a nullable column of every kind a condition compares.
#[table(name = "samples")]
pub struct Sample {
	pub id: Id<Self>,
	pub int: Option<i64>,
	pub int2: Option<i64>,
	pub float: Option<f64>,
	pub small: Option<f32>,
	pub text: Option<String>,
	pub text2: Option<String>,
	pub flag: Option<bool>,
	pub uuid: Option<Uuid>,
	pub at: Option<Timestamp>,
	pub at2: Option<Timestamp>,
	pub day: Option<NaiveDate>,
	pub bytes: Option<Vec<u8>>,
	pub doc: Option<Json<serde_json::Value>>,
	pub spot: Option<Geo<Point, Grid>>,
}

/// A planar coordinate system, in metres.
type Grid = Srid<3857>;

/// A request setting a condition compares a column with.
struct Level(i64);

impl Setting for Level {
	type Value = i64;

	const NAME: &'static str = "level";

	fn value(&self) -> i64 {
		self.0
	}
}

leptoskit_db::register_setting!(Level);

/// Rows for a condition to count.
#[table(name = "others")]
pub struct Other {
	pub id: Id<Self>,
	pub n: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
	Int,
	Float,
	Small,
	Text,
	Flag,
	Uuid,
	At,
	Day,
	Bytes,
	Doc,
}

const COLUMNS: [(&str, Kind); 13] = [
	("int", Kind::Int),
	("int2", Kind::Int),
	("float", Kind::Float),
	("small", Kind::Small),
	("text", Kind::Text),
	("text2", Kind::Text),
	("flag", Kind::Flag),
	("uuid", Kind::Uuid),
	("at", Kind::At),
	("at2", Kind::At),
	("day", Kind::Day),
	("bytes", Kind::Bytes),
	("doc", Kind::Doc),
];

/// Pairs of columns a condition compares with each other.
const PAIRS: [(&str, &str); 5] = [
	("int", "int2"),
	("text", "text2"),
	("at", "at2"),
	("float", "small"),
	("int", "int"),
];

const FLOATS: [f64; 12] = [
	-1.5,
	-0.0,
	0.0,
	0.1,
	0.5,
	1.0,
	0.100_000_001_490_116_12,
	2.5,
	1e300,
	f64::INFINITY,
	f64::NEG_INFINITY,
	f64::NAN,
];

const SMALL: [f32; 5] = [-1.5, 0.1, 0.5, f32::MAX, f32::NAN];

const TEXTS: [&str; 11] = ["", "a", "A", "b", "ab", "a ", "é", "Z", "%", "_", "ß"];

const PATTERNS: [&str; 6] = ["a%", "%", "_", "A", "%b", "é"];

fn instants() -> Vec<Timestamp> {
	let noon = Utc.with_ymd_and_hms(2026, 10, 30, 12, 0, 0).unwrap();
	vec![
		noon,
		noon + TimeDelta::milliseconds(1),
		noon + TimeDelta::microseconds(1),
		noon + TimeDelta::nanoseconds(1),
		noon - TimeDelta::days(1),
		Utc.with_ymd_and_hms(1970, 1, 1, 0, 0, 0).unwrap(),
		Utc.with_ymd_and_hms(9999, 12, 31, 23, 59, 59).unwrap(),
	]
}

fn days() -> Vec<NaiveDate> {
	[
		(1970, 1, 1),
		(2026, 10, 30),
		(2026, 10, 31),
		(1, 1, 1),
		(9999, 12, 31),
	]
	.into_iter()
	.map(|(year, month, day)| NaiveDate::from_ymd_opt(year, month, day).unwrap())
	.collect()
}

fn uuids() -> Vec<Uuid> {
	vec![
		Uuid::from_u128(1),
		Uuid::from_u128(2),
		Uuid::from_u128(1 << 64),
		Uuid::from_u128(u128::MAX),
	]
}

fn documents() -> Vec<serde_json::Value> {
	vec![
		serde_json::Value::Null,
		serde_json::json!(1),
		serde_json::json!("a"),
		serde_json::json!([1]),
	]
}

/// A value for a column of `kind`, as a condition would compare it with.
fn literal(kind: Kind) -> BoxedStrategy<Value> {
	match kind {
		Kind::Int => {
			prop_oneof![(-3_i64..=3), Just(i64::MIN), Just(i64::MAX)]
				.prop_map(Value::Int)
				.boxed()
		}
		Kind::Float => {
			prop::sample::select(FLOATS.to_vec())
				.prop_map(Value::Float)
				.boxed()
		}
		// Postgres binds a value compared with a `real` column as an `f32`:
		// `0.1` then equals `0.1_f32`, which as an `f64` it doesn't.
		Kind::Small => {
			prop_oneof![
				prop::sample::select(SMALL.to_vec()).prop_map(f64::from),
				prop::sample::select(vec![0.1, 0.5, 2.5, 1e300, f64::NAN]),
			]
			.prop_map(Value::Float)
			.boxed()
		}
		Kind::Text => {
			prop::sample::select(TEXTS.to_vec())
				.prop_map(|text| Value::Text(text.into()))
				.boxed()
		}
		Kind::Flag => any::<bool>().prop_map(Value::Bool).boxed(),
		Kind::Uuid => prop::sample::select(uuids()).prop_map(Value::Uuid).boxed(),
		Kind::At => {
			prop::sample::select(instants())
				.prop_map(Value::Timestamp)
				.boxed()
		}
		Kind::Day => prop::sample::select(days()).prop_map(Value::Date).boxed(),
		Kind::Bytes => {
			prop::sample::select(vec![vec![], vec![0], vec![0, 1], vec![1], vec![255]])
				.prop_map(Value::Bytes)
				.boxed()
		}
		Kind::Doc => {
			prop::sample::select(documents())
				.prop_map(Value::Json)
				.boxed()
		}
	}
}

/// A literal, or now and then `None`.
fn operand(kind: Kind) -> BoxedStrategy<Value> {
	prop_oneof![9 => literal(kind), 1 => Just(Value::Null)].boxed()
}

fn comparison() -> impl Strategy<Value = Comparison> {
	prop::sample::select(vec![
		Comparison::Equal,
		Comparison::NotEqual,
		Comparison::Greater,
		Comparison::GreaterOrEqual,
		Comparison::Less,
		Comparison::LessOrEqual,
	])
}

/// A column to compare, more often one whose values the databases treat
/// differently.
fn column() -> impl Strategy<Value = (&'static str, Kind)> {
	let mut columns = COLUMNS.to_vec();
	columns.extend([
		("small", Kind::Small),
		("small", Kind::Small),
		("at", Kind::At),
		("float", Kind::Float),
	]);
	prop::sample::select(columns)
}

fn table() -> TableName {
	Sample::table_name()
}

/// One condition on a sample, of any form `Expr` has outside links.
fn leaf() -> impl Strategy<Value = Expr> {
	let compare = (column(), comparison())
		.prop_flat_map(|((column, kind), comparison)| {
			operand(kind).prop_map(move |value| {
				Expr::Compare {
					column,
					comparison,
					value,
				}
			})
		})
		.boxed();
	let within = column()
		.prop_flat_map(|(column, kind)| {
			prop::collection::vec(operand(kind), 0..4)
				.prop_map(move |values| Expr::In { column, values })
		})
		.boxed();
	let null = (column(), any::<bool>())
		.prop_map(|((column, _), negated)| Expr::Null { column, negated })
		.boxed();
	let columns = (prop::sample::select(PAIRS.to_vec()), comparison())
		.prop_map(|((column, other), comparison)| {
			Expr::CompareColumn {
				column,
				comparison,
				table: table(),
				other,
			}
		})
		.boxed();
	let text = (
		prop::sample::select(vec!["text", "text2"]),
		prop::sample::select(PATTERNS.to_vec()),
		any::<bool>(),
	)
		.prop_map(|(column, pattern, like)| {
			if like {
				Expr::Like {
					column,
					pattern: pattern.into(),
				}
			} else {
				Expr::Contains {
					column,
					needle: pattern.into(),
				}
			}
		})
		.boxed();
	let count = (0_i64..3, 0_i64..3, any::<bool>())
		.prop_map(|(above, at_least, correlated)| {
			let cx = Context::new();
			let others = Other::query().filter(|other| {
				if correlated {
					other.n.eq(Sample::columns().int)
				} else {
					other.n.gt(above)
				}
			});
			cx.count(others).gte(at_least)
		})
		.boxed();

	let exists = (0_i64..4, any::<bool>())
		.prop_map(|(above, correlated)| {
			let cx = Context::new();
			cx.exists(Other::query().filter(|other| {
				if correlated {
					other.n.gt(Sample::columns().int2)
				} else {
					other.n.gt(above)
				}
			}))
		})
		.boxed();
	// The rows a link's visibility rule looks up.
	let looked_up = (0_i64..4, prop::sample::select(vec!["int", "int2"]))
		.prop_map(|(above, column)| {
			Expr::within(
				column,
				"n",
				Other::query().filter(|other| other.n.gte(above)),
			)
		})
		.boxed();
	// A request setting, which binds its value or `NULL` without one.
	let setting = (comparison(), prop::option::of(-3_i64..=3))
		.prop_map(|(comparison, level)| {
			let cx = match level {
				Some(level) => Context::new().with(Level(level)),
				None => Context::new(),
			};
			let int = Sample::columns().int;
			let level = cx.setting::<Level>();
			match comparison {
				Comparison::Equal => int.eq(level),
				Comparison::NotEqual => int.ne(level),
				Comparison::Greater => int.gt(level),
				Comparison::GreaterOrEqual => int.gte(level),
				Comparison::Less => int.lt(level),
				Comparison::LessOrEqual => int.lte(level),
			}
		})
		.boxed();
	let spatial = (-2_i32..=2, -2_i32..=2, 0_u8..3)
		.prop_map(|(x, y, kind)| {
			let spot = Sample::columns().spot;
			let corner = coord! { x: f64::from(x), y: f64::from(y) };
			match kind {
				0 => {
					spot.intersects(&Geo::<Rect, Grid>::new(Rect::new(
						corner,
						coord! { x: corner.x + 1.0, y: corner.y + 1.0 },
					)))
				}
				1 => spot.within_distance(&Geo::<Point, Grid>::new(Point::from(corner)), 1.5),
				_ => {
					spot.distance_to(&Geo::<Point, Grid>::new(Point::from(corner)))
						.lt(2.0)
				}
			}
		})
		.boxed();

	prop_oneof![
		4 => compare,
		2 => within,
		1 => null,
		2 => columns,
		1 => text,
		1 => count,
		1 => exists,
		1 => looked_up,
		2 => setting,
		1 => spatial,
		1 => any::<bool>().prop_map(Expr::Constant),
	]
}

fn condition() -> impl Strategy<Value = Expr> {
	leaf().prop_recursive(3, 12, 2, |inner| {
		prop_oneof![
			(inner.clone(), inner.clone()).prop_map(|(left, right)| left & right),
			(inner.clone(), inner.clone()).prop_map(|(left, right)| left | right),
			inner.prop_map(|condition| !condition),
		]
	})
}

/// A sample's values: `NewSample`, which proptest can print.
#[derive(Clone, Debug)]
struct Spec {
	int: Option<i64>,
	int2: Option<i64>,
	float: Option<f64>,
	small: Option<f32>,
	text: Option<String>,
	text2: Option<String>,
	flag: Option<bool>,
	uuid: Option<Uuid>,
	at: Option<Timestamp>,
	at2: Option<Timestamp>,
	day: Option<NaiveDate>,
	bytes: Option<Vec<u8>>,
	doc: Option<Json<serde_json::Value>>,
	spot: Option<(i32, i32)>,
}

impl Spec {
	fn row(self) -> NewSample {
		NewSample {
			int: self.int,
			int2: self.int2,
			float: self.float,
			small: self.small,
			text: self.text,
			text2: self.text2,
			flag: self.flag,
			uuid: self.uuid,
			at: self.at,
			at2: self.at2,
			day: self.day,
			bytes: self.bytes,
			doc: self.doc,
			spot: self
				.spot
				.map(|(x, y)| Geo::new(point! { x: f64::from(x), y: f64::from(y) })),
		}
	}
}

fn sample() -> impl Strategy<Value = Spec> {
	let float = prop::option::weighted(0.7, prop::sample::select(FLOATS.to_vec()));
	let small = prop::option::weighted(0.7, prop::sample::select(SMALL.to_vec()));
	let text = || {
		prop::option::weighted(
			0.7,
			prop::sample::select(TEXTS.to_vec()).prop_map(String::from),
		)
	};
	let at = || prop::option::weighted(0.7, prop::sample::select(instants()));
	let int = || {
		prop::option::weighted(
			0.7,
			prop_oneof![(-3_i64..=3), Just(i64::MIN), Just(i64::MAX)],
		)
	};

	(
		(int(), int(), float, small, text(), text()),
		(
			prop::option::weighted(0.7, any::<bool>()),
			prop::option::weighted(0.7, prop::sample::select(uuids())),
			at(),
			at(),
			prop::option::weighted(0.7, prop::sample::select(days())),
			prop::option::weighted(
				0.7,
				prop::sample::select(vec![vec![], vec![0], vec![0, 1], vec![1], vec![255]]),
			),
			prop::option::weighted(0.7, prop::sample::select(documents()).prop_map(Json)),
			prop::option::weighted(0.7, (-3_i32..=3, -3_i32..=3)),
		),
	)
		.prop_map(
			|(
				(int, int2, float, small, text, text2),
				(flag, uuid, at, at2, day, bytes, doc, spot),
			)| {
				Spec {
					int,
					int2,
					float,
					small,
					text,
					text2,
					flag,
					uuid,
					at,
					at2,
					day,
					bytes,
					doc,
					spot,
				}
			},
		)
}

/// What SQL decides for the one stored row: `TRUE`, `FALSE`, or `NULL`, or
/// nothing when the condition doesn't run on this database.
async fn sql_decides(db: &Db, condition: &Expr) -> Option<Tri> {
	let holds = Sample::query()
		.filter(|_| condition.clone())
		.count(db)
		.await
		.ok()?;
	let fails = Sample::query()
		.filter(|_| !condition.clone())
		.count(db)
		.await
		.ok()?;

	Some(match (holds, fails) {
		(1, 0) => Tri::True,
		(0, 1) => Tri::False,
		(0, 0) => Tri::Unknown,
		other => panic!("one row both meets and fails a condition: {other:?}"),
	})
}

/// How many answers `eval` gave, how many were definite, and how often SQL
/// ran a condition holding each kind of `Expr`.
#[derive(Default)]
struct Tally {
	answers: AtomicUsize,
	definite: AtomicUsize,
	kinds: Mutex<BTreeMap<String, usize>>,
}

/// Every kind of `Expr` a query can run, which the conditions must cover.
const KINDS: [&str; 14] = [
	"Compare",
	"CompareColumn",
	"CompareSetting",
	"In",
	"Null",
	"Contains",
	"Like",
	"And",
	"Or",
	"Not",
	"Count",
	"Within",
	"Spatial",
	"Constant",
];

/// Adds the kind of `condition` and of every condition inside it.
fn kinds(condition: &Expr, found: &mut BTreeMap<String, usize>) {
	let debug = format!("{condition:?}");
	let kind: String = debug
		.chars()
		.take_while(char::is_ascii_alphanumeric)
		.collect();
	*found.entry(kind).or_default() += 1;

	match condition {
		Expr::And(left, right) | Expr::Or(left, right) => {
			kinds(left, found);
			kinds(right, found);
		}
		Expr::Not(inner) => kinds(inner, found),
		_ => {}
	}
}

/// How a condition is decided on one row: [`Expr::eval`], or a broken
/// stand-in that the property must catch.
type Eval = fn(&Expr, &BTreeMap<String, Value>) -> Tri;

/// Whether the property checks only soundness, or that every definite
/// answer is SQL's as well.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Strictness {
	Sound,
	Exact,
}

async fn agrees_with_sql(
	db: &Db,
	eval: Eval,
	strictness: Strictness,
	tally: &Tally,
	sample: Spec,
	conditions: Vec<Expr>,
) -> Result<(), TestCaseError> {
	Sample::query().destroy(db).await.unwrap();
	sample.row().insert(db).await.unwrap();
	// The row as the database stored it, which is what a change log holds.
	let stored = Sample::query().one(db).await.unwrap();
	let row: BTreeMap<String, Value> = Sample::COLUMNS
		.iter()
		.map(ToString::to_string)
		.zip(stored.to_values().unwrap())
		.collect();

	for condition in conditions {
		let Some(sql) = sql_decides(db, &condition).await else {
			continue;
		};
		let eval = eval(&condition, &row);
		tally.answers.fetch_add(1, Ordering::Relaxed);
		kinds(
			&condition,
			&mut tally.kinds.lock().unwrap_or_else(PoisonError::into_inner),
		);

		// Soundness: false only where SQL is false.
		prop_assert!(
			!(sql == Tri::True && eval == Tri::False),
			"eval says false where SQL says true on {:?}\nfor {:?}",
			condition,
			row
		);

		if eval != Tri::Unknown {
			tally.definite.fetch_add(1, Ordering::Relaxed);
		}

		if eval != Tri::Unknown && strictness == Strictness::Exact {
			prop_assert_eq!(
				eval,
				sql,
				"eval disagrees with SQL on {:?}\nfor {:?}",
				condition,
				row
			);
		}
	}

	Ok(())
}

async fn create_schema(db: &Db) {
	let dialect = db.dialect();
	let schema = leptoskit_db::schema(dialect).unwrap();
	let empty = Schema::empty(dialect);
	let plan = leptoskit_db_schema::diff(&empty, &schema, &mut RequireHints).unwrap();
	let migration = leptoskit_db_schema::render(&plan, &empty, &schema);
	db.run_script(&migration.sql()).await.unwrap();

	for n in 0..3 {
		NewOther { n }.insert(db).await.unwrap();
	}
}

/// Runs the property with `eval` for `cases` cases, returning what failed.
fn check(
	db: &Db,
	runtime: &tokio::runtime::Runtime,
	cases: u32,
	eval: Eval,
	strictness: Strictness,
) -> Result<Tally, String> {
	let tally = Tally::default();
	let mut runner = TestRunner::new(Config {
		cases,
		failure_persistence: None,
		..Config::default()
	});
	let strategy = (sample(), prop::collection::vec(condition(), 1..8));
	runner
		.run(&strategy, |(sample, conditions)| {
			runtime.block_on(agrees_with_sql(
				db, eval, strictness, &tally, sample, conditions,
			))
		})
		.map_err(|error| error.to_string())?;
	Ok(tally)
}

fn run_property(db: &Db, runtime: &tokio::runtime::Runtime, cases: u32) {
	let eval: Eval = |condition, row| condition.eval(row);
	let tally = match check(db, runtime, cases, eval, Strictness::Exact) {
		Ok(tally) => tally,
		Err(error) => panic!("{error}"),
	};

	// Soundness alone would allow `eval` to know nothing: it must decide a
	// good share of conditions itself. About half are decidable here; the
	// rest count rows, match text, read settings, measure shapes, or meet a
	// `NULL`.
	let answers = tally.answers.load(Ordering::Relaxed);
	let definite = tally.definite.load(Ordering::Relaxed);
	eprintln!("eval decided {definite} of {answers} conditions");
	assert!(
		definite * 4 > answers,
		"eval decided only {definite} of {answers} conditions"
	);

	// Every kind of condition ran on this database, so each was checked.
	let kinds = tally
		.kinds
		.into_inner()
		.unwrap_or_else(PoisonError::into_inner);
	eprintln!("conditions checked, by kind: {kinds:?}");
	for kind in KINDS {
		assert!(
			kinds.get(kind).copied().unwrap_or_default() > 0,
			"no condition holding `{kind}` ran"
		);
	}
}

/// A broken `eval` that guesses false whenever the real one can't tell: the
/// unsound shortcut a matcher might take.
fn unknown_is_false(condition: &Expr, row: &BTreeMap<String, Value>) -> Tri {
	match condition.eval(row) {
		Tri::Unknown => Tri::False,
		decided => decided,
	}
}

fn sqlite(runtime: &tokio::runtime::Runtime) -> Db {
	runtime.block_on(async {
		let db = Db::connect("sqlite::memory:").await.unwrap();
		create_schema(&db).await;
		db
	})
}

fn postgres(runtime: &tokio::runtime::Runtime, cluster: &support::postgres::PostgresCluster) -> Db {
	runtime.block_on(async {
		let db = Db::connect(&cluster.database()).await.unwrap();
		create_schema(&db).await;
		db
	})
}

#[test]
fn eval_agrees_with_sqlite() {
	let runtime = tokio::runtime::Runtime::new().unwrap();
	run_property(&sqlite(&runtime), &runtime, 512);
}

#[test]
fn eval_agrees_with_postgres() {
	let Some(cluster) = support::postgres::PostgresCluster::start_with("postgis") else {
		return;
	};
	let runtime = tokio::runtime::Runtime::new().unwrap();
	run_property(&postgres(&runtime, &cluster), &runtime, 512);
}

/// The property isn't vacuous: an `eval` that says false where it can't
/// tell fails it on both databases, and the soundness check alone catches
/// it.
#[test]
fn an_unsound_eval_fails_the_property() {
	let runtime = tokio::runtime::Runtime::new().unwrap();
	let mut databases = vec![sqlite(&runtime)];
	let cluster = support::postgres::PostgresCluster::start_with("postgis");

	if let Some(cluster) = &cluster {
		databases.push(postgres(&runtime, cluster));
	}

	for db in databases {
		let Err(error) = check(&db, &runtime, 256, unknown_is_false, Strictness::Sound) else {
			panic!("an unsound eval passed on {:?}", db.dialect());
		};
		assert!(
			error.contains("eval says false where SQL says true"),
			"{error}"
		);
		assert!(
			check(&db, &runtime, 256, unknown_is_false, Strictness::Exact).is_err(),
			"an unsound eval passed the exact check on {:?}",
			db.dialect()
		);
	}
}

/// Where the two databases decide the same condition differently, `eval`
/// can't know which one it's answering for, so it doesn't answer.
#[tokio::test(flavor = "multi_thread")]
async fn conditions_the_databases_disagree_on_are_unknown() {
	let noon = Utc.with_ymd_and_hms(2026, 10, 30, 12, 0, 0).unwrap();
	let row = Spec {
		int: None,
		int2: None,
		float: None,
		small: Some(0.1),
		text: Some("é".into()),
		text2: None,
		flag: None,
		uuid: None,
		at: Some(noon),
		at2: None,
		day: None,
		bytes: None,
		doc: None,
		spot: None,
	};
	let columns = Sample::columns();
	// `(condition, on SQLite, on Postgres)`.
	let cases = [
		// Postgres binds `0.1` as the `real` it's compared with.
		(columns.small.eq(0.1_f32), Tri::True, Tri::True),
		(
			Expr::Compare {
				column: "small",
				comparison: Comparison::Equal,
				value: Value::Float(0.1),
			},
			Tri::False,
			Tri::True,
		),
		// SQLite keeps milliseconds; Postgres keeps microseconds.
		(
			columns.at.eq(noon + TimeDelta::microseconds(1)),
			Tri::True,
			Tri::False,
		),
	];

	let mut databases = vec![(Db::connect("sqlite::memory:").await.unwrap(), 1)];
	let cluster = support::postgres::PostgresCluster::start_with("postgis");

	if let Some(cluster) = &cluster {
		databases.push((Db::connect(&cluster.database()).await.unwrap(), 2));
	}

	for (db, position) in databases {
		create_schema(&db).await;
		row.clone().row().insert(&db).await.unwrap();
		let stored = Sample::query().one(&db).await.unwrap();
		let values: BTreeMap<String, Value> = Sample::COLUMNS
			.iter()
			.map(ToString::to_string)
			.zip(stored.to_values().unwrap())
			.collect();

		for (index, case) in cases.iter().enumerate() {
			let condition = &case.0;
			let expected = if position == 1 { case.1 } else { case.2 };
			assert_eq!(
				sql_decides(&db, condition).await,
				Some(expected),
				"case {index} on {:?}",
				db.dialect()
			);
		}

		for (condition, ..) in &cases[1..] {
			assert_eq!(condition.eval(&values), Tri::Unknown, "{condition:?}");
		}
	}
}
