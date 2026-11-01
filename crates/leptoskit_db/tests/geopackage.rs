//! GeoPackage metadata on SQLite (`[db.geo] geopackage = true`), and
//! spatial indexes surviving migrations that rebuild and rename their table
//! with rows in it, run with the real geospatial functions.

#![cfg(all(feature = "sqlite", feature = "geo"))]

use leptoskit_db::Connection;
use leptoskit_db::Db;
use leptoskit_db::Dialect;
use leptoskit_db::Id;
use leptoskit_db::Read;
use leptoskit_db::Statement;
use leptoskit_db::Value;
use leptoskit_db::geo::Geo;
use leptoskit_db::geo::Point;
use leptoskit_db::geo::Srid;
use leptoskit_db::geo::point;
use leptoskit_db::table;
use leptoskit_db_schema::Check;
use leptoskit_db_schema::CheckRule;
use leptoskit_db_schema::RequireHints;
use leptoskit_db_schema::Schema;
use leptoskit_db_schema::TableName;

/// A survey marker on the British National Grid.
#[table(name = "markers")]
pub struct Marker {
	pub id: Id<Self>,
	pub height: i32,
	#[db(geo(index))]
	pub at: Geo<Point, Srid<27700>>,
}

async fn migrate(db: &Db, from: &Schema, to: &Schema) {
	let plan = leptoskit_db_schema::diff(from, to, &mut RequireHints).unwrap();
	let migration = leptoskit_db_schema::render(&plan, from, to);
	db.run_script(&migration.sql()).await.unwrap();
}

async fn rows(db: &Db, sql: &str) -> Vec<Vec<Value>> {
	db.fetch(Statement {
		sql: sql.into(),
		args: Vec::new(),
	})
	.await
	.unwrap()
}

async fn count(db: &Db, sql: &str) -> i64 {
	match rows(db, sql).await[0][0] {
		Value::Int(count) => count,
		ref other => panic!("{other:?}"),
	}
}

fn schema() -> Schema {
	leptoskit_db::with_geopackage(leptoskit_db::schema(Dialect::Sqlite).unwrap(), true).unwrap()
}

#[tokio::test]
async fn geopackage_registers_each_layer() {
	let db = Db::connect("sqlite::memory:").await.unwrap();
	migrate(&db, &Schema::empty(Dialect::Sqlite), &schema()).await;

	assert_eq!(
		rows(
			&db,
			"SELECT table_name, column_name, geometry_type_name, srs_id, z, m FROM \
			 gpkg_geometry_columns"
		)
		.await,
		vec![vec![
			Value::Text("markers".into()),
			Value::Text("at".into()),
			Value::Text("POINT".into()),
			Value::Int(27700),
			Value::Int(0),
			Value::Int(0),
		]]
	);
	assert_eq!(
		count(
			&db,
			"SELECT count(*) FROM gpkg_contents WHERE data_type = 'features'"
		)
		.await,
		1
	);
	assert_eq!(
		count(
			&db,
			"SELECT count(*) FROM gpkg_spatial_ref_sys WHERE srs_id IN (-1, 0, 4326, 27700)"
		)
		.await,
		4
	);
	assert_eq!(count(&db, "PRAGMA application_id").await, 0x4750_4B47);
	assert_eq!(count(&db, "PRAGMA user_version").await, 10_400);

	// Rows are GeoPackage geometries, which GIS tools read as written.
	let marker = NewMarker {
		height: 120,
		at: Geo::new(point!(x: 530_000.0, y: 180_000.0)),
	}
	.insert(&db)
	.await
	.unwrap();
	let stored = rows(&db, "SELECT at FROM markers").await;
	let Value::Bytes(bytes) = &stored[0][0] else {
		panic!("{stored:?}");
	};
	assert_eq!(&bytes[..2], b"GP");
	assert_eq!(&bytes[4..8], &27700_i32.to_le_bytes());
	assert_eq!(Marker::get(&db, marker.id).await.unwrap().at, marker.at);

	// More than one geometry column, or Postgres, can't be a GeoPackage.
	let mut two = schema();
	two.tables[0].columns.push(leptoskit_db_schema::Column::new(
		"other",
		leptoskit_db_schema::ColumnType::Geometry {
			shape: leptoskit_db_schema::GeoShape::Point,
			srid: 27700,
		},
	));
	assert!(leptoskit_db::with_geopackage(two, true).is_err());
	assert!(
		leptoskit_db::with_geopackage(leptoskit_db::schema(Dialect::Postgres).unwrap(), true)
			.is_err()
	);
	assert!(
		!leptoskit_db::with_geopackage(leptoskit_db::schema(Dialect::Postgres).unwrap(), false)
			.unwrap()
			.geopackage
	);
}

#[tokio::test]
async fn spatial_indexes_survive_rebuilds_and_renames_with_rows() {
	let db = Db::connect("sqlite::memory:").await.unwrap();
	let first = schema();
	migrate(&db, &Schema::empty(Dialect::Sqlite), &first).await;

	for (index, x) in [100.0, 200.0, 300.0].into_iter().enumerate() {
		NewMarker {
			height: index as i32,
			at: Geo::new(point!(x: x, y: 0.0)),
		}
		.insert(&db)
		.await
		.unwrap();
	}

	let near = |x: f64| {
		let db = db.clone();
		async move {
			Marker::query()
				.filter(|marker| {
					marker
						.at
						.within_distance(&Geo::new(point!(x: x, y: 0.0)), 1.0)
				})
				.count(&db)
				.await
				.unwrap()
		}
	};

	// A new check rebuilds the table; the index keeps every row's box and
	// its triggers come back.
	let mut rebuilt = first.clone();
	rebuilt.tables[0].checks.push(Check {
		name: "markers_height_range".into(),
		rule: CheckRule::Range {
			column: "height".into(),
			min: Some(0.0),
			max: Some(10.0),
		},
	});
	migrate(&db, &first, &rebuilt).await;
	assert_eq!(count(&db, "SELECT count(*) FROM rtree_markers_at").await, 3);
	assert_eq!(near(200.0).await, 1);
	NewMarker {
		height: 4,
		at: Geo::new(point!(x: 400.0, y: 0.0)),
	}
	.insert(&db)
	.await
	.unwrap();
	assert_eq!(near(400.0).await, 1);
	assert_eq!(count(&db, "SELECT count(*) FROM rtree_markers_at").await, 4);

	// Renaming the table recreates the index under the new name, filled
	// from the rows, and moves the GeoPackage layer.
	let mut renamed = rebuilt.clone();
	renamed.tables[0].name = TableName::new("benchmarks");
	renamed.tables[0].renamed_from = Some(TableName::new("markers"));
	migrate(&db, &rebuilt, &renamed).await;
	assert_eq!(
		count(&db, "SELECT count(*) FROM rtree_benchmarks_at").await,
		4
	);
	assert_eq!(
		count(
			&db,
			"SELECT count(*) FROM sqlite_schema WHERE name LIKE 'rtree_markers_at%'"
		)
		.await,
		0
	);
	assert_eq!(
		count(
			&db,
			"SELECT count(*) FROM gpkg_geometry_columns WHERE table_name = 'benchmarks'"
		)
		.await,
		1
	);
	// The new triggers index raw writes.
	db.execute(Statement {
		sql: "DELETE FROM benchmarks WHERE height = 0".into(),
		args: Vec::new(),
	})
	.await
	.unwrap();
	assert_eq!(
		count(&db, "SELECT count(*) FROM rtree_benchmarks_at").await,
		3
	);
	assert_eq!(
		count(&db, "SELECT count(*) FROM rtree_benchmarks_at_rows").await,
		3
	);

	// Turning GeoPackage off drops its tables.
	renamed.tables[0].renamed_from = None;
	let mut plain = renamed.clone();
	plain.geopackage = false;
	migrate(&db, &renamed, &plain).await;
	assert_eq!(
		count(
			&db,
			"SELECT count(*) FROM sqlite_schema WHERE name LIKE 'gpkg_%'"
		)
		.await,
		0
	);
	assert_eq!(count(&db, "PRAGMA application_id").await, 0);
}
