//! Vector columns against both databases: every kind round trips, the
//! SQLite `vec0` tables follow every write, raw SQL included, and one
//! fixture suite of nearest-neighbour searches, for every distance, runs on
//! pgvector and on sqlite-vec against distances measured in Rust, with
//! filters, policies, and soft deletes choosing the rows.

#![cfg(all(feature = "postgres", feature = "sqlite", feature = "vector"))]

mod support;

use leptoskit_db::Connection;
use leptoskit_db::Context;
use leptoskit_db::Db;
use leptoskit_db::DbError;
use leptoskit_db::Dialect;
use leptoskit_db::Expr;
use leptoskit_db::Id;
use leptoskit_db::Read;
use leptoskit_db::Statement;
use leptoskit_db::Table;
use leptoskit_db::Timestamp;
use leptoskit_db::Value;
use leptoskit_db::policy;
use leptoskit_db::table;
use leptoskit_db::vector::BitVector;
use leptoskit_db::vector::HalfVector;
use leptoskit_db::vector::Vector;
use leptoskit_db::vector::f16;
use leptoskit_db_schema::ColumnType;
use leptoskit_db_schema::RequireHints;
use leptoskit_db_schema::Schema;
use leptoskit_db_schema::VectorDistance;
use leptoskit_db_schema::VectorElement;
use leptoskit_db_schema::VectorIndexKind;
use leptoskit_db_schema::VectorOptions;

/// The author making the request.
struct Viewer(String);

/// A document: everyone sees a published one, and its author the rest.
#[table(name = "documents", public)]
#[policy(select, |doc, cx| doc.published.eq(true) | cx.global(|viewer: &Viewer| doc.author.eq(viewer.0.clone())))]
#[policy(insert, update, delete, |_, _| Expr::always())]
pub struct Document {
	pub id: Id<Self>,
	pub author: String,
	pub title: String,
	pub body: Option<String>,
	pub published: bool,
	#[db(vector(distance = cosine, index = hnsw, source(title, body)))]
	pub embedding: Option<Vector<3>>,
	#[db(deleted_at)]
	pub deleted_at: Option<Timestamp>,
}

/// The same four dimensions, once per distance.
#[table(name = "measures", public)]
pub struct Measure {
	pub id: Id<Self>,
	pub label: String,
	#[db(vector(distance = l2))]
	pub l2: Vector<4>,
	#[db(vector(distance = cosine, index = hnsw))]
	pub cosine: Vector<4>,
	#[db(vector(distance = inner_product, index = ivfflat))]
	pub inner: HalfVector<4>,
	#[db(vector(distance = l1, index = hnsw))]
	pub taxicab: Vector<4>,
	#[db(vector(distance = hamming, index = ivfflat))]
	pub bits: BitVector<16>,
	#[db(vector(distance = jaccard, index = hnsw))]
	pub sets: BitVector<16>,
}

/// A vector clients see, and one with no options at all.
#[table(name = "codes", public)]
pub struct Code {
	pub id: Id<Self>,
	#[db(public, vector(distance = hamming))]
	pub code: BitVector<8>,
	pub plain: Option<Vector<2>>,
}

/// A pin keyed by two columns in a namespace.
#[table(name = "pins", namespace = "maps")]
pub struct Pin {
	#[db(key)]
	pub board: i32,
	#[db(key)]
	pub slot: i32,
	#[db(vector(distance = l2))]
	pub at: Vector<2>,
}

async fn create_schema(db: &Db) {
	let dialect = db.dialect();
	let schema = leptoskit_db::schema(dialect).unwrap();
	let empty = Schema::empty(dialect);
	let plan = leptoskit_db_schema::diff(&empty, &schema, &mut RequireHints).unwrap();
	let migration = leptoskit_db_schema::render(&plan, &empty, &schema);
	db.run_script(&migration.sql()).await.unwrap();
}

async fn on_both<F, Fut>(check: F)
where
	F: Fn(Db) -> Fut,
	Fut: Future<Output = ()>,
{
	let db = Db::connect("sqlite::memory:").await.unwrap();
	create_schema(&db).await;
	check(db).await;

	if let Some(cluster) = support::postgres::PostgresCluster::start_with("vector") {
		let db = Db::connect(&cluster.database()).await.unwrap();
		create_schema(&db).await;
		check(db).await;
	}
}

/// Pseudo-random floats from `seed`, the same on every run.
fn floats<const N: usize>(seed: u64) -> [f32; N] {
	let mut state = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
	std::array::from_fn(|_| {
		state = state
			.wrapping_mul(6_364_136_223_846_793_005)
			.wrapping_add(1_442_695_040_888_963_407);
		((state >> 40) as f32 / (1_u64 << 24) as f32) * 2.0 - 1.0
	})
}

/// Sixteen bits, the first `count` set.
fn first_bits(count: usize) -> BitVector<16> {
	BitVector::from_bits(std::array::from_fn(|index| index < count))
}

fn dot(left: &[f32], right: &[f32]) -> f64 {
	left.iter()
		.zip(right)
		.map(|(left, right)| f64::from(*left) * f64::from(*right))
		.sum()
}

/// A stored measure's vectors, to measure in Rust.
fn vectors(row: &Measure) -> NewMeasure {
	NewMeasure {
		label: row.label.clone(),
		l2: row.l2.clone(),
		cosine: row.cosine.clone(),
		inner: row.inner.clone(),
		taxicab: row.taxicab.clone(),
		bits: row.bits.clone(),
		sets: row.sets.clone(),
	}
}

/// The distance pgvector and sqlite-vec measure, measured in Rust.
fn measured(distance: VectorDistance, left: &NewMeasure, right: &NewMeasure) -> f64 {
	let floats = |measure: &NewMeasure| -> Vec<f32> {
		match distance {
			VectorDistance::L2 => measure.l2.as_slice().to_vec(),
			VectorDistance::Cosine => measure.cosine.as_slice().to_vec(),
			VectorDistance::InnerProduct => measure.inner.to_vector().as_slice().to_vec(),
			_ => measure.taxicab.as_slice().to_vec(),
		}
	};
	let bits = |measure: &NewMeasure| -> Vec<u8> {
		match distance {
			VectorDistance::Hamming => measure.bits.as_bytes().to_vec(),
			_ => measure.sets.as_bytes().to_vec(),
		}
	};
	let (a, b) = (floats(left), floats(right));

	match distance {
		VectorDistance::L2 => {
			a.iter()
				.zip(&b)
				.map(|(a, b)| (f64::from(*a) - f64::from(*b)).powi(2))
				.sum::<f64>()
				.sqrt()
		}
		VectorDistance::Cosine => 1.0 - dot(&a, &b) / (dot(&a, &a).sqrt() * dot(&b, &b).sqrt()),
		VectorDistance::InnerProduct => -dot(&a, &b),
		VectorDistance::L1 => {
			a.iter()
				.zip(&b)
				.map(|(a, b)| (f64::from(*a) - f64::from(*b)).abs())
				.sum()
		}
		VectorDistance::Hamming => {
			bits(left)
				.iter()
				.zip(bits(right))
				.map(|(a, b)| f64::from((a ^ b).count_ones()))
				.sum()
		}
		VectorDistance::Jaccard => {
			let (a, b) = (bits(left), bits(right));
			let both: u32 = a.iter().zip(&b).map(|(a, b)| (a & b).count_ones()).sum();
			let either: u32 = a.iter().zip(&b).map(|(a, b)| (a | b).count_ones()).sum();
			1.0 - f64::from(both) / f64::from(either)
		}
	}
}

fn measure(label: String, seed: u64, ones: usize) -> NewMeasure {
	NewMeasure {
		label,
		l2: Vector::new(floats(seed)),
		cosine: Vector::new(floats(seed + 1)),
		inner: HalfVector::from(&Vector::new(floats(seed + 2))),
		taxicab: Vector::new(floats(seed + 3)),
		bits: first_bits(ones),
		sets: first_bits(ones),
	}
}

#[test]
fn the_macro_declares_vector_columns() {
	let schema = leptoskit_db::schema(Dialect::Postgres).unwrap();
	let table = |name: &str| {
		schema
			.tables
			.iter()
			.find(|table| table.name.name == name)
			.unwrap()
	};
	let documents = table("documents");

	assert_eq!(
		documents.column("embedding").unwrap().ty,
		ColumnType::Vector {
			element: VectorElement::Float32,
			dimensions: 3
		}
	);
	assert_eq!(
		documents.vectors,
		vec![
			VectorOptions::new("embedding", Some(VectorDistance::Cosine))
				.indexed(VectorIndexKind::Hnsw)
				.from_sources(&["title", "body"])
		]
	);
	let source = documents.column("lk_embedding_source").unwrap();
	assert!(source.nullable);
	assert_eq!(source.ty, ColumnType::TEXT);
	assert!(!Document::COLUMNS.contains(&"lk_embedding_source"));

	let measures = table("measures");
	assert_eq!(
		measures.column("inner").unwrap().ty,
		ColumnType::Vector {
			element: VectorElement::Float16,
			dimensions: 4
		}
	);
	assert_eq!(
		measures.column("bits").unwrap().ty,
		ColumnType::Vector {
			element: VectorElement::Bit,
			dimensions: 16
		}
	);
	assert_eq!(measures.vectors.len(), 6);

	// Vector columns are private unless they say otherwise; a vector with
	// no options is still recorded, so `lk db check` sees it.
	let codes = table("codes");
	assert_eq!(
		codes
			.vectors
			.iter()
			.map(|vector| (vector.column.as_str(), vector.distance, vector.public))
			.collect::<Vec<_>>(),
		[
			("code", Some(VectorDistance::Hamming), true),
			("plain", None, false)
		]
	);
	assert_eq!(schema.extensions(), vec!["vector"]);

	let public = serde_json::to_value(PublicDocument {
		id: Id::new(),
		author: "ada".into(),
		title: "Hello".into(),
		body: None,
		published: true,
		deleted_at: None,
	})
	.unwrap();
	assert!(public.get("embedding").is_none());
	let code = serde_json::to_value(PublicCode {
		id: Id::new(),
		code: BitVector::from_bits([true; 8]),
	})
	.unwrap();
	assert_eq!(code["code"], "11111111");
}

#[tokio::test]
async fn every_kind_of_vector_round_trips() {
	on_both(|db| {
		async move {
			let stored = measure("one".into(), 7, 5).insert(&db).await.unwrap();
			let read = Measure::get(&db, stored.id).await.unwrap();
			assert_eq!(read.l2, stored.l2);
			assert_eq!(read.inner, stored.inner);
			assert_eq!(read.bits, first_bits(5));

			let mut read = read;
			read.cosine = Vector::new([1.0, -2.0, 0.5, 0.25]);
			read.inner = HalfVector::new([f16::from_f32(0.125); 4]);
			read.save(&db).await.unwrap();
			let again = Measure::get(&db, stored.id).await.unwrap();
			assert_eq!(again.cosine.as_slice(), &[1.0, -2.0, 0.5, 0.25]);
			assert_eq!(again.inner.to_vector().as_slice(), &[0.125; 4]);

			let pin = NewPin {
				board: 1,
				slot: 2,
				at: Vector::new([3.0, 4.0]),
			}
			.insert(&db)
			.await
			.unwrap();
			let read = Pin::query()
				.filter(|p| p.board.eq(1) & p.slot.eq(2))
				.one(&db)
				.await
				.unwrap();
			assert_eq!(read.at, pin.at);

			// Values that aren't finite never reach the database.
			let refused = NewMeasure {
				l2: Vector::new([f32::NAN, 0.0, 0.0, 0.0]),
				..measure("nan".into(), 8, 1)
			}
			.insert(&db)
			.await;
			assert!(
				matches!(&refused, Err(DbError::Encode(error)) if error.to_string().contains("isn't finite")),
				"{refused:?}"
			);
		}
	})
	.await;
}

/// How many vectors and rows a SQLite `vec0` table and its row numbers
/// hold.
async fn indexed(db: &Db, table: &str) -> (i64, i64) {
	let count = |name: String| {
		async move {
			match db
				.fetch(Statement {
					sql: format!("SELECT count(*) FROM \"{name}\""),
					args: Vec::new(),
				})
				.await
				.unwrap()[0][0]
			{
				Value::Int(count) => count,
				ref other => panic!("{other:?}"),
			}
		}
	};
	(
		count(format!("{table}_lk_vec_embedding")).await,
		count(format!("{table}_lk_vec_embedding_rows")).await,
	)
}

#[tokio::test]
async fn sqlite_keeps_its_vec0_tables_current_through_every_write() {
	let db = Db::connect("sqlite::memory:").await.unwrap();
	create_schema(&db).await;
	let document = |title: &str, embedding: Option<[f32; 3]>| {
		NewDocument {
			author: "ada".into(),
			title: title.into(),
			body: None,
			published: true,
			embedding: embedding.map(Vector::new),
		}
	};

	let first = document("first", Some([1.0, 0.0, 0.0]))
		.insert(&db)
		.await
		.unwrap();
	let mut second = document("second", None).insert(&db).await.unwrap();
	assert_eq!(indexed(&db, "documents").await, (1, 2));

	second.embedding = Some(Vector::new([0.0, 1.0, 0.0]));
	second.save(&db).await.unwrap();
	assert_eq!(indexed(&db, "documents").await, (2, 2));

	Document::update()
		.filter(|doc| doc.id.eq(first.id))
		.set(|doc| doc.embedding, None)
		.run(&db)
		.await
		.unwrap();
	assert_eq!(indexed(&db, "documents").await, (1, 2));

	// A soft delete keeps the vector; searches leave the row out.
	second.delete(&db).await.unwrap();
	assert_eq!(indexed(&db, "documents").await, (1, 2));
	second.destroy(&db).await.unwrap();
	assert_eq!(indexed(&db, "documents").await, (0, 1));

	// Raw SQL is indexed too.
	db.execute(Statement {
		sql: "INSERT INTO documents (id, author, title, published, embedding) VALUES (?1, \
		      'grace', 'raw', 1, ?2)"
			.into(),
		args: vec![
			Value::Bytes(Id::<Document>::new().uuid().as_bytes().to_vec()),
			Value::Bytes(
				[0_f32, 0.0, 1.0]
					.iter()
					.flat_map(|value| value.to_le_bytes())
					.collect(),
			),
		],
	})
	.await
	.unwrap();
	assert_eq!(indexed(&db, "documents").await, (1, 2));
	let nearest = Document::query()
		.nearest(|doc| doc.embedding, &Vector::new([0.0, 0.1, 1.0]))
		.limit(1)
		.all(&db)
		.await
		.unwrap();
	assert_eq!(nearest[0].title, "raw");
}

/// Twelve measures, each with a different number of bits set, so Hamming
/// and Jaccard distances to the target never tie.
async fn measures(db: &Db) -> Vec<Measure> {
	let mut stored = Vec::new();

	for index in 0..12 {
		stored.push(
			measure(format!("m{index:02}"), index as u64 * 11, index + 2)
				.insert(db)
				.await
				.unwrap(),
		);
	}

	stored
}

#[tokio::test]
async fn nearest_orders_by_every_distance_as_measured_in_rust() {
	on_both(|db| {
		async move {
			let stored = measures(&db).await;
			let target = measure("target".into(), 1_000, 16);

			for distance in VectorDistance::ALL {
				// With a limit, SQLite searches the `vec0` table where it
				// has one; without, it measures every row.
				for limit in [Some(5), None] {
					assert_nearest(&db, &stored, &target, distance, limit).await;
				}
			}
		}
	})
	.await;
}

/// Checks the measures nearest `target` by `distance` against the
/// distances measured in Rust. Postgres's indexes are approximate, so its
/// order is checked when asked for exact rows, and its index for as many
/// rows as asked.
async fn assert_nearest(
	db: &Db,
	stored: &[Measure],
	target: &NewMeasure,
	distance: VectorDistance,
	limit: Option<u64>,
) {
	let mut expected: Vec<(f64, String)> = stored
		.iter()
		.map(|row| (measured(distance, target, &vectors(row)), row.label.clone()))
		.collect();
	expected.sort_by(|left, right| left.partial_cmp(right).unwrap());
	let wanted = limit.unwrap_or(expected.len() as u64) as usize;
	let context = format!("{} by {distance}, limit {limit:?}", db.dialect());

	let approximate = search(db, distance, target, limit, false).await;
	assert_eq!(approximate.len(), wanted, "{context}");

	let found = search(db, distance, target, limit, true).await;
	assert_eq!(
		found
			.iter()
			.map(|(label, _)| label.as_str())
			.collect::<Vec<_>>(),
		expected[..wanted]
			.iter()
			.map(|(_, label)| label.as_str())
			.collect::<Vec<_>>(),
		"{context}"
	);

	for ((_, found), (expected, label)) in found.iter().zip(&expected) {
		assert!(
			(f64::from(*found) - expected).abs() < 1e-4,
			"{context}: {label} is {found}, measured {expected}"
		);
	}
}

/// The labels and distances of the measures nearest `target` by
/// `distance`, ties broken by label.
async fn search(
	db: &Db,
	distance: VectorDistance,
	target: &NewMeasure,
	limit: Option<u64>,
	exact: bool,
) -> Vec<(String, f32)> {
	let query = Measure::query().order_by(|m| m.label.asc());
	let limited = |rows: Vec<(Measure, f32)>| {
		rows.into_iter()
			.map(|(row, distance)| (row.label, distance))
			.collect()
	};

	macro_rules! nearest {
		($column:ident) => {{
			let search = query
				.nearest(|m| m.$column, &target.$column)
				.with_distance();

			let search = match limit {
				Some(limit) => search.limit(limit),
				None => search,
			};
			let search = if exact { search.exact() } else { search };
			limited(search.all(db).await.unwrap())
		}};
	}

	match distance {
		VectorDistance::L2 => nearest!(l2),
		VectorDistance::Cosine => nearest!(cosine),
		VectorDistance::InnerProduct => nearest!(inner),
		VectorDistance::L1 => nearest!(taxicab),
		VectorDistance::Hamming => nearest!(bits),
		VectorDistance::Jaccard => nearest!(sets),
	}
}

/// Thirty documents, alternately published, by two authors, spread around
/// the unit circle so each is a different distance from `[1, 0, 0]`.
async fn documents(db: &Db) -> Vec<Document> {
	let mut stored = Vec::new();

	for index in 0..30 {
		let angle = index as f32 * 0.1;
		stored.push(
			NewDocument {
				author: if index % 3 == 0 { "ada" } else { "grace" }.into(),
				title: format!("d{index:02}"),
				body: None,
				published: index % 2 == 0,
				embedding: Some(Vector::new([angle.cos(), angle.sin(), 0.0])),
			}
			.insert(db)
			.await
			.unwrap(),
		);
	}

	stored
}

fn titles(rows: &[Document]) -> Vec<String> {
	rows.iter().map(|row| row.title.clone()).collect()
}

#[tokio::test]
async fn filters_policies_and_soft_deletes_choose_the_rows_searched() {
	on_both(|db| {
		async move {
			let stored = documents(&db).await;
			let target = Vector::new([1.0, 0.0, 0.0]);
			let expected = |keep: &dyn Fn(&Document) -> bool, limit: usize| -> Vec<String> {
				stored
					.iter()
					.filter(|doc| keep(doc))
					.take(limit)
					.map(|doc| doc.title.clone())
					.collect()
			};
			let anonymous = db.with_context(Context::new());
			let ada = db.with_context(Context::new().with(Viewer("ada".into())));

			// The nearest published documents, past the unpublished ones that
			// are nearer.
			let found = Document::query()
				.nearest(|doc| doc.embedding, &target)
				.limit(5)
				.all(&anonymous)
				.await
				.unwrap();
			assert_eq!(titles(&found), expected(&|doc| doc.published, 5));

			let found = Document::query()
				.nearest(|doc| doc.embedding, &target)
				.limit(5)
				.all(&ada)
				.await
				.unwrap();
			assert_eq!(
				titles(&found),
				expected(&|doc| doc.published || doc.author == "ada", 5)
			);

			// A filter keeping few rows still finds as many as asked for.
			let found = Document::query()
				.filter(|doc| doc.author.eq("ada"))
				.nearest(|doc| doc.embedding, &target)
				.offset(1)
				.limit(4)
				.all(&ada)
				.await
				.unwrap();
			assert_eq!(
				titles(&found),
				expected(&|doc| doc.author == "ada", 5)[1..].to_vec()
			);

			// Soft-deleted rows are left out unless asked for.
			let mut nearest_first = Document::get(&db, stored[0].id).await.unwrap();
			nearest_first.delete(&db).await.unwrap();
			let nearest = |with_deleted: bool| {
				let query = Document::query()
					.nearest(|doc| doc.embedding, &target)
					.limit(1);
				let query = if with_deleted {
					query.with_deleted()
				} else {
					query
				};

				query.first(&db)
			};
			assert_eq!(nearest(false).await.unwrap().unwrap().title, "d02");
			assert_eq!(nearest(true).await.unwrap().unwrap().title, "d00");
		}
	})
	.await;
}

#[tokio::test]
async fn a_search_needs_a_distance() {
	on_both(|db| {
		async move {
			let refused = Code::query()
				.nearest(|code| code.plain, &Vector::new([1.0, 0.0]))
				.all(&db)
				.await;
			assert!(
				matches!(&refused, Err(DbError::Encode(error)) if error.to_string().contains("has no distance")),
				"{refused:?}"
			);
		}
	})
	.await;
}

#[tokio::test]
async fn postgres_indexes_find_every_row_asked_for() {
	let Some(cluster) = support::postgres::PostgresCluster::start_with("vector") else {
		return;
	};
	let db = Db::connect(&cluster.database()).await.unwrap();
	create_schema(&db).await;

	for index in 0..400_u64 {
		measure(format!("m{index:03}"), index, (index % 16) as usize + 1)
			.insert(&db)
			.await
			.unwrap();
	}

	// With sequential scans off, the indexes answer, and HNSW alone would
	// stop at its 40 candidates and IVFFlat at its first list.
	let tx = db.begin().await.unwrap();
	tx.execute(Statement {
		sql: "SET LOCAL enable_seqscan = off".into(),
		args: Vec::new(),
	})
	.await
	.unwrap();
	let target = Vector::new(floats::<4>(9_999));
	let unfiltered = Measure::query()
		.nearest(|m| m.cosine, &target)
		.limit(60)
		.all(&tx)
		.await
		.unwrap();
	assert_eq!(unfiltered.len(), 60);
	let filtered = Measure::query()
		.filter(|m| m.label.ilike("%7"))
		.nearest(|m| m.inner, &HalfVector::from(&target))
		.limit(15)
		.all(&tx)
		.await
		.unwrap();
	assert_eq!(filtered.len(), 15);
	assert!(filtered.iter().all(|m| m.label.ends_with('7')));

	tx.rollback().await.unwrap();
}

/// An article, searched by its words and by its embedding, which is made
/// from its title and body.
#[table(name = "articles", public)]
#[policy(select, |article, cx| article.published.eq(true) | cx.global(|viewer: &Viewer| article.author.eq(viewer.0.clone())))]
#[policy(insert, update, delete, |_, _| Expr::always())]
pub struct Article {
	pub id: Id<Self>,
	pub author: String,
	#[db(search(weight = A))]
	pub title: String,
	#[db(search(weight = B))]
	pub body: Option<String>,
	pub published: bool,
	#[db(vector(distance = cosine, index = hnsw, source(title, body)))]
	pub embedding: Option<Vector<3>>,
}

/// A model that counts three words, so its vectors can be checked: `rust`,
/// `passkeys`, and `sqlite`, plus a little of everything.
#[derive(Default)]
struct Counter {
	calls: std::sync::atomic::AtomicUsize,
	short: bool,
}

#[derive(Debug, thiserror::Error)]
#[error("the model is down")]
struct Down;

fn counted(text: &str) -> Vector<3> {
	let count = |word: &str| text.to_lowercase().matches(word).count() as f32;
	Vector::new([
		count("rust") + 0.1,
		count("passkeys") + 0.1,
		count("sqlite") + 0.1,
	])
}

impl leptoskit_db::vector::Embedder<Vector<3>> for Counter {
	type Error = Down;

	fn embed(&self, texts: &[String]) -> impl Future<Output = Result<Vec<Vector<3>>, Down>> + Send {
		self.calls
			.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
		let mut vectors: Vec<Vector<3>> = texts.iter().map(|text| counted(text)).collect();

		if self.short {
			vectors.pop();
		}

		std::future::ready(Ok(vectors))
	}
}

async fn article(
	db: &Db,
	title: &str,
	body: Option<&str>,
	embedding: Option<Vector<3>>,
) -> Article {
	NewArticle {
		author: "ada".into(),
		title: title.into(),
		body: body.map(Into::into),
		published: true,
		embedding,
	}
	.insert(db)
	.await
	.unwrap()
}

async fn stale_titles(db: &Db) -> Vec<String> {
	let mut titles: Vec<String> = Article::stale_embeddings()
		.all(db)
		.await
		.unwrap()
		.into_iter()
		.map(|article| article.title)
		.collect();
	titles.sort();
	titles
}

#[tokio::test]
async fn writes_to_a_vectors_sources_leave_it_stale_until_it_is_made_again() {
	on_both(|db| {
		async move {
			let missing = article(&db, "missing", None, None).await;
			let mut edited =
				article(&db, "edited", Some("rust"), Some(counted("edited\n\nrust"))).await;
			let mut moved = article(
				&db,
				"moved",
				Some("sqlite"),
				Some(counted("moved\n\nsqlite")),
			)
			.await;
			assert_eq!(stale_titles(&db).await, ["missing"]);

			// A new vector records the fingerprint of its sources, the same
			// one Rust computes on either database.
			let fingerprint = db
				.fetch(Statement {
					sql: "SELECT lk_embedding_source FROM articles WHERE title = 'edited'".into(),
					args: Vec::new(),
				})
				.await
				.unwrap();
			assert_eq!(
				fingerprint[0][0],
				Value::Text(leptoskit_db_schema::source_fingerprint(&[
					Some("edited"),
					Some("rust")
				]))
			);

			// Changing a source makes the vector stale; anything else doesn't.
			edited.body = Some("rust and passkeys".into());
			edited.save(&db).await.unwrap();
			moved.author = "grace".into();
			moved.save(&db).await.unwrap();
			assert_eq!(stale_titles(&db).await, ["edited", "missing"]);

			// Raw SQL too.
			db.execute(Statement {
				sql: "UPDATE articles SET title = 'renamed' WHERE title = 'moved'".into(),
				args: Vec::new(),
			})
			.await
			.unwrap();
			assert_eq!(stale_titles(&db).await, ["edited", "missing", "renamed"]);

			// Fresh rows can be left out of a search.
			let fresh = Article::query()
				.filter(|a| !a.embedding.is_stale())
				.count(&db)
				.await
				.unwrap();
			assert_eq!(fresh, 0);

			// The job embeds each stale row's sources, once.
			let model = Counter::default();
			let embedded = Article::stale_embeddings()
				.limit(2)
				.embed_stale(|a| a.embedding, &model, &db)
				.await
				.unwrap();
			assert_eq!(embedded, 2);
			let embedded = Article::stale_embeddings()
				.embed_stale(|a| a.embedding, &model, &db)
				.await
				.unwrap();
			assert_eq!(embedded, 1);
			assert_eq!(stale_titles(&db).await, Vec::<String>::new());
			assert_eq!(
				Article::stale_embeddings()
					.embed_stale(|a| a.embedding, &model, &db)
					.await
					.unwrap(),
				0
			);
			assert_eq!(model.calls.load(std::sync::atomic::Ordering::Relaxed), 2);

			let edited = Article::get(&db, edited.id).await.unwrap();
			assert_eq!(
				edited.embedding,
				Some(counted("edited\n\nrust and passkeys"))
			);
			let missing = Article::get(&db, missing.id).await.unwrap();
			assert_eq!(missing.embedding, Some(counted("missing")));

			// Making the same vector again still freshens the row.
			let mut again = Article::get(&db, edited.id).await.unwrap();
			again.title = "edited again".into();
			again.save(&db).await.unwrap();
			db.execute(Statement {
				sql: "UPDATE articles SET body = NULL WHERE title = 'edited again'".into(),
				args: Vec::new(),
			})
			.await
			.unwrap();
			assert_eq!(stale_titles(&db).await, ["edited again"]);
			assert_eq!(
				Article::stale_embeddings()
					.embed_stale(|a| a.embedding, &model, &db)
					.await
					.unwrap(),
				1
			);
			assert_eq!(stale_titles(&db).await, Vec::<String>::new());
		}
	})
	.await;
}

#[tokio::test]
async fn the_embedding_job_reports_a_model_that_fails() {
	on_both(|db| {
		async move {
			article(&db, "one", None, None).await;
			article(&db, "two", None, None).await;
			let short = Counter {
				short: true,
				..Counter::default()
			};
			let refused = Article::stale_embeddings()
				.embed_stale(|a| a.embedding, &short, &db)
				.await;
			assert!(
				matches!(&refused, Err(DbError::Embedder(error)) if error.to_string() == "the embedder made 1 vectors for 2 texts"),
				"{refused:?}"
			);
			assert_eq!(stale_titles(&db).await, ["one", "two"]);

			// A column without sources can't be stale.
			let refused = Measure::query()
				.filter(|m| m.cosine.is_stale())
				.all(&db)
				.await;
			assert!(
				matches!(&refused, Err(DbError::Encode(error)) if error.to_string().contains("isn't made from sources")),
				"{refused:?}"
			);
		}
	})
	.await;
}

#[tokio::test]
async fn hybrid_search_fuses_the_text_and_vector_rankings() {
	on_both(|db| {
		async move {
			let texts = [
				("passkeys in rust", "rust rust passkeys"),
				("rust", "sqlite and rust"),
				("sqlite", "sqlite sqlite"),
				("passkeys", "passkeys everywhere"),
				("rust passkeys", "rust"),
				("cooking", "nothing here"),
			];

			for (title, body) in texts {
				article(
					&db,
					title,
					Some(body),
					Some(counted(&format!("{title}\n\n{body}"))),
				)
				.await;
			}

			let mut hidden = article(
				&db,
				"rust secrets",
				Some("rust passkeys"),
				Some(counted("rust passkeys")),
			)
			.await;
			hidden.published = false;
			hidden.author = "grace".into();
			hidden.save(&db).await.unwrap();

			let target = counted("rust passkeys");
			let text: Vec<String> = Article::query()
				.search("rust")
				.all(&db)
				.await
				.unwrap()
				.into_iter()
				.map(|article| article.title)
				.collect();
			let vector: Vec<String> = Article::query()
				.nearest(|a| a.embedding, &target)
				.exact()
				.all(&db)
				.await
				.unwrap()
				.into_iter()
				.map(|article| article.title)
				.collect();
			assert!(!text.contains(&"rust secrets".to_string()));

			// Reciprocal rank fusion, measured in Rust from the two rankings.
			let rank = |list: &[String], title: &str| list.iter().position(|other| other == title);
			let mut expected: Vec<(f64, usize, String)> = vector
				.iter()
				.map(|title| {
					let ranks = [rank(&text, title), rank(&vector, title)];
					let score = ranks
						.iter()
						.flatten()
						.map(|rank| 1.0 / (61.0 + *rank as f64))
						.sum();
					let best = ranks.iter().flatten().min().copied().unwrap_or(usize::MAX);
					(score, best, title.clone())
				})
				.collect();
			expected.sort_by(|left, right| right.0.total_cmp(&left.0).then(left.1.cmp(&right.1)));

			let hits = Article::query()
				.hybrid(|h| h.text("rust").vector(|a| a.embedding, &target))
				.limit(4)
				.hits(&db)
				.await
				.unwrap();
			assert_eq!(
				hits.iter()
					.map(|hit| hit.row.title.as_str())
					.collect::<Vec<_>>(),
				expected[..4]
					.iter()
					.map(|(_, _, title)| title.as_str())
					.collect::<Vec<_>>(),
				"{}",
				db.dialect()
			);

			for hit in &hits {
				assert_eq!(
					hit.text_rank,
					rank(&text, &hit.row.title).map(|rank| rank + 1)
				);
				assert_eq!(
					hit.vector_rank,
					rank(&vector, &hit.row.title).map(|rank| rank + 1)
				);
				assert!(hit.distance.is_some());
			}

			// The viewer's policy applies to both rankings.
			let ada = db.with_context(Context::new().with(Viewer("grace".into())));
			let mine = Article::query()
				.hybrid(|h| h.text("secrets").vector(|a| a.embedding, &target))
				.all(&ada)
				.await
				.unwrap();
			assert!(mine.iter().any(|article| article.title == "rust secrets"));

			// Text alone, and a filter.
			let only_text = Article::query()
				.hybrid(|h| h.text("rust"))
				.all(&db)
				.await
				.unwrap();
			assert_eq!(
				only_text
					.into_iter()
					.map(|article| article.title)
					.collect::<Vec<_>>(),
				text
			);
			let filtered = Article::query()
				.hybrid(|h| h.vector(|a| a.embedding, &target))
				.filter(|a| a.title.ilike("%sqlite%"))
				.all(&db)
				.await
				.unwrap();
			assert_eq!(
				filtered
					.into_iter()
					.map(|article| article.title)
					.collect::<Vec<_>>(),
				["sqlite"]
			);

			assert!(Article::query().hybrid(|h| h).all(&db).await.is_err());
		}
	})
	.await;
}
