//! Geospatial columns against both databases: storage round trips, one
//! fixture suite of the portable operations run on PostGIS and on SQLite's
//! layer, spatial indexes kept current by every write, raw SQL included,
//! checks in the database, PostGIS-only operations, and policies filtering
//! what spatial queries find.

#![cfg(all(feature = "postgres", feature = "sqlite", feature = "geo"))]

mod support;

use std::collections::BTreeMap;

use geo::Distance;
use geo::Geodesic;
use geo::Relate;
use leptoskit_db::Connection;
use leptoskit_db::Context;
use leptoskit_db::Db;
use leptoskit_db::DbError;
use leptoskit_db::Dialect;
use leptoskit_db::Expr;
use leptoskit_db::Id;
use leptoskit_db::Read;
use leptoskit_db::ReadEntry;
use leptoskit_db::ReadKind;
use leptoskit_db::Shape;
use leptoskit_db::Statement;
use leptoskit_db::Table;
use leptoskit_db::Timestamp;
use leptoskit_db::Tracked;
use leptoskit_db::Tri;
use leptoskit_db::Value;
use leptoskit_db::geo::Geo;
use leptoskit_db::geo::Geography;
use leptoskit_db::geo::Geometry;
use leptoskit_db::geo::GeometryCollection;
use leptoskit_db::geo::LineString;
use leptoskit_db::geo::Meters;
use leptoskit_db::geo::MultiPolygon;
use leptoskit_db::geo::Point;
use leptoskit_db::geo::Polygon;
use leptoskit_db::geo::Rect;
use leptoskit_db::geo::Srid;
use leptoskit_db::geo::Wkb;
use leptoskit_db::geo::coord;
use leptoskit_db::geo::line_string;
use leptoskit_db::geo::point;
use leptoskit_db::geo::polygon;
use leptoskit_db::policy;
use leptoskit_db::table;
use leptoskit_db_schema::CheckRule;
use leptoskit_db_schema::ColumnType;
use leptoskit_db_schema::GeoIndex;
use leptoskit_db_schema::GeoShape;
use leptoskit_db_schema::RequireHints;
use leptoskit_db_schema::Schema;
use proptest::prelude::*;
use proptest::test_runner::Config;
use proptest::test_runner::TestCaseError;
use proptest::test_runner::TestRunner;

/// The British National Grid, in metres.
type Grid = Srid<27700>;

/// The owner making the request.
struct Viewer(Id<Owner>);

/// Someone who owns places.
#[table(name = "owners")]
pub struct Owner {
	pub id: Id<Self>,
	pub name: String,
}

/// A place: everyone sees a public one, and its owner sees the rest.
#[table(name = "places", audit)]
#[policy(select, |place, cx| place.public.eq(true) | cx.global(|viewer: &Viewer| place.owner_id.eq(viewer.0)))]
#[policy(insert, update, delete, |_, _| Expr::always())]
pub struct Place {
	pub id: Id<Self>,
	#[db(backlink = places)]
	pub owner_id: Id<Owner>,
	pub name: String,
	pub public: bool,
	#[db(geo(index))]
	pub location: Geography<Point>,
	#[db(geo(index, valid))]
	pub area: Option<Geography<MultiPolygon>>,
	#[db(geo(index))]
	pub spot: Option<Geo<Point, Grid>>,
	#[db(deleted_at)]
	pub deleted_at: Option<Timestamp>,
}

/// A planar shape of any kind, kept twice: once without an index and once
/// with one, so every query can be checked both ways.
#[table(name = "plots")]
pub struct Plot {
	pub id: Id<Self>,
	pub name: String,
	pub shape: Geo<Geometry, Grid>,
	#[db(geo(index))]
	pub indexed: Geo<Geometry, Grid>,
}

/// Every other kind of column a shape can be.
#[table(name = "routes")]
pub struct Route {
	pub id: Id<Self>,
	pub path: Geography<LineString>,
	pub raw: Wkb<Geography<LineString>>,
	pub zone: Option<Geography<Polygon>>,
	pub bounds: Option<Geo<Rect, Srid<3857>>>,
	pub collection: Option<Geo<GeometryCollection, Srid<3857>>>,
}

/// A pin on a board, keyed by two columns in a namespace, indexed.
#[table(name = "pins", namespace = "maps")]
pub struct Pin {
	#[db(key)]
	pub board: i32,
	#[db(key)]
	pub slot: i32,
	#[db(geo(index))]
	pub at: Geography<Point>,
}

/// An owner and their places, in one statement.
#[derive(Shape, Debug, PartialEq)]
#[db(from = Owner)]
pub struct Holdings {
	pub name: String,
	#[db(via = places, order_by = |place| place.name.asc())]
	pub places: Vec<Spot>,
}

/// A place's shapes, nested.
#[derive(Shape, Debug, PartialEq)]
#[db(from = Place)]
pub struct Spot {
	pub name: String,
	pub location: Geography<Point>,
	pub spot: Option<Geo<Point, Grid>>,
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

	if let Some(cluster) = support::postgres::PostgresCluster::start_with("postgis") {
		let db = Db::connect(&cluster.database()).await.unwrap();
		create_schema(&db).await;
		check(db).await;
	}
}

/// A live place, whose shape is copied into the change log. Anyone may
/// read a public one.
#[table(name = "live_places", live)]
#[policy(select, |place, _| place.public.eq(true))]
pub struct LivePlace {
	pub id: Id<Self>,
	pub public: bool,
	pub at: Geography<Point>,
}

#[tokio::test]
async fn a_live_table_with_a_shape_is_captured_and_checked() {
	on_both(async |db| {
		let system = db.system("a live shape");
		let mut anyone = db.changes().await.unwrap();
		let mut log = system.change_log(None).await.unwrap();
		let place = NewLivePlace {
			public: true,
			at: trafalgar(),
		}
		.insert(&system)
		.await
		.unwrap();
		NewLivePlace {
			public: false,
			at: trafalgar(),
		}
		.insert(&system)
		.await
		.unwrap();

		// The policy check reads the image; only the public place is heard.
		let heard = tokio::time::timeout(std::time::Duration::from_secs(10), anyone.next())
			.await
			.unwrap()
			.unwrap();
		assert_eq!(
			heard,
			leptoskit_db::live::Invalidation::Rows(vec![leptoskit_db::live::ChangedRows {
				table: "live_places".into(),
				keys: vec![vec![place.id.to_string()]],
			}])
		);

		// The log keeps the shape as the database wrote it.
		let leptoskit_db::live::LogEntry::Commit(commit) = log.next().await.unwrap() else {
			panic!("expected a commit");
		};
		let new = commit.changes[0].new.as_ref().unwrap();
		assert_ne!(new["at"], serde_json::Value::Null);
		assert_eq!(new["public"], serde_json::Value::Bool(true));
	})
	.await;
}

/// Trafalgar Square, London.
fn trafalgar() -> Geography<Point> {
	Geography(point!(x: -0.1281, y: 51.5080))
}

fn square(min_x: f64, min_y: f64, size: f64) -> Polygon {
	polygon![
		(x: min_x, y: min_y),
		(x: min_x + size, y: min_y),
		(x: min_x + size, y: min_y + size),
		(x: min_x, y: min_y + size),
		(x: min_x, y: min_y),
	]
}

async fn owner(db: &Db, name: &str) -> Id<Owner> {
	NewOwner { name: name.into() }.insert(db).await.unwrap().id
}

async fn place(db: &Db, owner_id: Id<Owner>, name: &str, location: Geography<Point>) -> Place {
	NewPlace {
		owner_id,
		name: name.into(),
		public: true,
		location,
		area: None,
		spot: None,
	}
	.insert(db)
	.await
	.unwrap()
}

fn names<T>(rows: &[T], name: impl Fn(&T) -> &str) -> Vec<String> {
	let mut names: Vec<String> = rows.iter().map(|row| name(row).to_string()).collect();
	names.sort();
	names
}

#[test]
fn the_macro_declares_geospatial_columns() {
	let schema = leptoskit_db::schema(Dialect::Postgres).unwrap();
	let places = schema
		.tables
		.iter()
		.find(|table| table.name.name == "places")
		.unwrap();

	assert_eq!(
		places.column("location").unwrap().ty,
		ColumnType::Geography {
			shape: GeoShape::Point
		}
	);
	assert!(places.column("area").unwrap().nullable);
	assert_eq!(
		places.column("spot").unwrap().ty,
		ColumnType::Geometry {
			shape: GeoShape::Point,
			srid: 27700
		}
	);
	assert_eq!(
		places.geo_indexes,
		vec![
			GeoIndex::new("location"),
			GeoIndex::new("area"),
			GeoIndex::new("spot")
		]
	);
	assert!(places.checks.iter().any(|check| {
		check.rule
			== CheckRule::ValidGeometry {
				column: "area".into(),
			}
	}));
	assert_eq!(Place::GEO_INDEXED, &["location", "area", "spot"]);
	assert_eq!(Plot::GEO_INDEXED, &["indexed"]);
	assert_eq!(Route::GEO_INDEXED, &[] as &[&str]);

	assert_eq!(schema.extensions(), vec!["postgis"]);

	let routes = schema
		.tables
		.iter()
		.find(|table| table.name.name == "routes")
		.unwrap();
	let shape = |name: &str| routes.column(name).unwrap().ty.geo().unwrap();
	assert_eq!(shape("raw"), (GeoShape::LineString, 4326));
	assert_eq!(shape("bounds"), (GeoShape::Polygon, 3857));
	assert_eq!(shape("collection"), (GeoShape::GeometryCollection, 3857));
}

#[tokio::test]
async fn every_kind_of_shape_round_trips() {
	on_both(|db| {
		async move {
			let ada = owner(&db, "ada").await;
			let area = MultiPolygon(vec![square(-0.13, 51.50, 0.01), square(-0.11, 51.50, 0.01)]);
			let stored = NewPlace {
				owner_id: ada,
				name: "gallery".into(),
				public: true,
				location: trafalgar(),
				area: Some(Geography(area.clone())),
				spot: Some(Geo::new(point!(x: 530_000.0, y: 180_000.0))),
			}
			.insert(&db)
			.await
			.unwrap();
			let read = Place::get(&db, stored.id).await.unwrap();
			assert_eq!(read.location, trafalgar());
			assert_eq!(read.area, Some(Geography(area)));
			assert_eq!(
				read.spot,
				Some(Geo::new(point!(x: 530_000.0, y: 180_000.0)))
			);

			let path = line_string![(x: -0.1, y: 51.5), (x: -0.2, y: 51.6), (x: -0.3, y: 51.5)];
			let bounds = Rect::new(coord! { x: 0.0, y: 0.0 }, coord! { x: 10.0, y: 20.0 });
			let collection = GeometryCollection(vec![
				Geometry::Point(point!(x: 1.0, y: 2.0)),
				Geometry::LineString(line_string![(x: 0.0, y: 0.0), (x: 3.0, y: 4.0)]),
			]);
			let route = NewRoute {
				path: Geography(path.clone()),
				raw: Wkb(Geography(path.clone())),
				zone: Some(Geography(square(-0.2, 51.4, 0.3))),
				bounds: Some(Geo::new(bounds)),
				collection: Some(Geo::new(collection.clone())),
			}
			.insert(&db)
			.await
			.unwrap();
			let read = Route::get(&db, route.id).await.unwrap();
			assert_eq!(read.path, Geography(path.clone()));
			assert_eq!(read.raw, Wkb(Geography(path)));
			assert_eq!(read.bounds, Some(Geo::new(bounds)));
			assert_eq!(read.collection, Some(Geo::new(collection)));

			// Updates and empty values.
			let mut read = read;
			read.zone = None;
			read.collection = Some(Geo::new(GeometryCollection(Vec::new())));
			read.save(&db).await.unwrap();
			let again = Route::get(&db, route.id).await.unwrap();
			assert_eq!(again.zone, None);
			assert_eq!(
				again.collection,
				Some(Geo::new(GeometryCollection(Vec::new())))
			);

			// Any shape in a `Geometry` column.
			for shape in [
				Geometry::Point(point!(x: 1.0, y: 1.0)),
				Geometry::Polygon(square(0.0, 0.0, 5.0)),
				Geometry::MultiPoint(vec![point!(x: 1.0, y: 1.0), point!(x: 2.0, y: 2.0)].into()),
			] {
				let plot = NewPlot {
					name: "any".into(),
					shape: Geo::new(shape.clone()),
					indexed: Geo::new(shape.clone()),
				}
				.insert(&db)
				.await
				.unwrap();
				assert_eq!(
					Plot::get(&db, plot.id).await.unwrap().shape,
					Geo::new(shape)
				);
			}
		}
	})
	.await;
}

#[tokio::test]
async fn shapes_nest_in_one_statement() {
	on_both(|db| {
		async move {
			let ada = owner(&db, "ada").await;
			let mut gallery = place(&db, ada, "gallery", trafalgar()).await;
			gallery.spot = Some(Geo::new(point!(x: 530_000.0, y: 180_000.0)));
			gallery.save(&db).await.unwrap();
			place(
				&db,
				ada,
				"museum",
				Geography(point!(x: -0.1270, y: 51.5194)),
			)
			.await;

			let holdings: Holdings = Owner::query()
				.filter(|owner| owner.id.eq(ada))
				.shape()
				.one(&db)
				.await
				.unwrap();
			assert_eq!(
				holdings,
				Holdings {
					name: "ada".into(),
					places: vec![
						Spot {
							name: "gallery".into(),
							location: trafalgar(),
							spot: Some(Geo::new(point!(x: 530_000.0, y: 180_000.0))),
						},
						Spot {
							name: "museum".into(),
							location: Geography(point!(x: -0.1270, y: 51.5194)),
							spot: None,
						},
					],
				}
			);
		}
	})
	.await;
}

/// One case of the shared fixture suite: a planar query and the plots it
/// must find, on both databases, with and without the index.
struct Fixture {
	name: &'static str,
	query: fn(&PlotColumns, bool) -> Expr,
	expected: &'static [&'static str],
}

fn column(plots: &PlotColumns, indexed: bool) -> leptoskit_db::Col<Plot, Geo<Geometry, Grid>> {
	if indexed { plots.indexed } else { plots.shape }
}

fn grid<T>(shape: T) -> Geo<T, Grid> {
	Geo::new(shape)
}

/// The plots: a field (a 100 m square at the origin), a barn inside it, a
/// shed touching its east side, a pond with a hole, a road crossing the
/// field, a well on its north edge, and a far tower.
fn plots() -> Vec<(&'static str, Geometry)> {
	let pond = Polygon::new(
		square(300.0, 0.0, 100.0).exterior().clone(),
		vec![square(325.0, 25.0, 50.0).exterior().clone()],
	);

	vec![
		("field", square(0.0, 0.0, 100.0).into()),
		("barn", square(10.0, 10.0, 20.0).into()),
		("shed", square(100.0, 40.0, 20.0).into()),
		("pond", pond.into()),
		(
			"road",
			line_string![(x: -50.0, y: 50.0), (x: 150.0, y: 50.0)].into(),
		),
		("well", point!(x: 50.0, y: 100.0).into()),
		("tower", point!(x: 5_000.0, y: 5_000.0).into()),
	]
}

fn fixtures() -> Vec<Fixture> {
	vec![
		Fixture {
			name: "intersects a point inside the field",
			query: |plots, indexed| {
				column(plots, indexed).intersects(&grid(point!(x: 50.0, y: 20.0)))
			},
			expected: &["field"],
		},
		Fixture {
			name: "intersects a box over the east edge",
			query: |plots, indexed| {
				column(plots, indexed).intersects(&grid(Rect::new(
					coord! { x: 90.0, y: 45.0 },
					coord! { x: 110.0, y: 55.0 },
				)))
			},
			expected: &["field", "road", "shed"],
		},
		Fixture {
			name: "contains a point: the field's boundary isn't inside it",
			query: |plots, indexed| {
				column(plots, indexed).contains(&grid(point!(x: 50.0, y: 100.0)))
			},
			expected: &["well"],
		},
		Fixture {
			name: "contains the barn",
			query: |plots, indexed| {
				column(plots, indexed).contains(&grid(square(10.0, 10.0, 20.0)))
			},
			expected: &["barn", "field"],
		},
		Fixture {
			name: "the hole isn't part of the pond",
			query: |plots, indexed| {
				column(plots, indexed).intersects(&grid(point!(x: 350.0, y: 50.0)))
			},
			expected: &[],
		},
		Fixture {
			name: "within the field",
			query: |plots, indexed| column(plots, indexed).within(&grid(square(0.0, 0.0, 100.0))),
			expected: &["barn", "field"],
		},
		Fixture {
			name: "touches the field",
			query: |plots, indexed| column(plots, indexed).touches(&grid(square(0.0, 0.0, 100.0))),
			expected: &["shed", "well"],
		},
		Fixture {
			name: "disjoint from the field",
			query: |plots, indexed| column(plots, indexed).disjoint(&grid(square(0.0, 0.0, 100.0))),
			expected: &["pond", "tower"],
		},
		Fixture {
			name: "within 210 m of the field's corner",
			query: |plots, indexed| {
				column(plots, indexed).within_distance(&grid(point!(x: 0.0, y: 0.0)), 210.0)
			},
			expected: &["barn", "field", "road", "shed", "well"],
		},
		Fixture {
			name: "within 1 m of the pond's far edge",
			query: |plots, indexed| {
				column(plots, indexed).within_distance(&grid(point!(x: 401.0, y: 50.0)), 1.0)
			},
			expected: &["pond"],
		},
		Fixture {
			name: "distance to a point beyond the shed, on the road",
			query: |plots, indexed| {
				column(plots, indexed)
					.distance_to(&grid(point!(x: 130.0, y: 50.0)))
					.lt(15.0)
			},
			expected: &["road", "shed"],
		},
		Fixture {
			name: "area larger than 5000",
			query: |plots, indexed| column(plots, indexed).area().gt(5_000.0),
			expected: &["field", "pond"],
		},
		Fixture {
			name: "length of 200",
			query: |plots, indexed| {
				column(plots, indexed).length().gte(199.999)
					& column(plots, indexed).length().lte(200.001)
			},
			expected: &["road"],
		},
	]
}

#[tokio::test]
async fn the_fixture_suite_agrees_on_both_databases() {
	on_both(|db| {
		async move {
			for (name, shape) in plots() {
				NewPlot {
					name: name.into(),
					shape: grid(shape.clone()),
					indexed: grid(shape),
				}
				.insert(&db)
				.await
				.unwrap();
			}

			for fixture in fixtures() {
				for indexed in [false, true] {
					let found = Plot::query()
						.filter(|plots| (fixture.query)(plots, indexed))
						.all(&db)
						.await
						.unwrap_or_else(|error| panic!("{}: {error}", fixture.name));
					assert_eq!(
						names(&found, |plot| &plot.name),
						fixture.expected,
						"{} on {} ({})",
						fixture.name,
						db.dialect(),
						if indexed { "indexed" } else { "unindexed" }
					);
				}
			}
		}
	})
	.await;
}

#[tokio::test]
async fn geography_measures_on_the_spheroid_alike() {
	on_both(|db| {
		async move {
			let ada = owner(&db, "ada").await;
			let places = [
				("gallery", trafalgar()),
				("museum", Geography(point!(x: -0.1270, y: 51.5194))),
				("tower", Geography(point!(x: -0.0761, y: 51.5081))),
				("paris", Geography(point!(x: 2.3522, y: 48.8566))),
			];

			for (name, location) in places.clone() {
				place(&db, ada, name, location).await;
			}

			// Within a radius, by the geodesic distance.
			let near = Place::query()
				.filter(|p| p.location.within_distance(&trafalgar(), Meters(3_700.0)))
				.all(&db)
				.await
				.unwrap();
			assert_eq!(names(&near, |p| &p.name), ["gallery", "museum", "tower"]);

			// Nearest first, with the distance, on both.
			let nearest = Place::query()
				.nearest(|p| p.location, &trafalgar())
				.with_distance()
				.limit(3)
				.all(&db)
				.await
				.unwrap();
			let order: Vec<&str> = nearest.iter().map(|(p, _)| p.name.as_str()).collect();
			assert_eq!(order, ["gallery", "museum", "tower"]);

			for (place, distance) in &nearest {
				let expected = Geodesic.distance(trafalgar().0, place.location.0);
				assert!(
					(distance.0 - expected).abs() < 1e-6,
					"{}: {} vs {expected} on {}",
					place.name,
					distance.0,
					db.dialect()
				);
			}

			let distances = Place::query()
				.filter(|p| p.location.distance_to(&trafalgar()).gt(300_000.0))
				.all(&db)
				.await
				.unwrap();
			assert_eq!(names(&distances, |p| &p.name), ["paris"]);

			// Areas and lengths in square metres and metres.
			let mut gallery = Place::query()
				.filter(|p| p.name.eq("gallery"))
				.one(&db)
				.await
				.unwrap();
			gallery.area = Some(Geography(MultiPolygon(vec![square(-0.13, 51.50, 0.01)])));
			gallery.save(&db).await.unwrap();
			let area = |bound: f64| {
				Place::query()
					.filter(move |p| p.area.area().gt(bound))
					.count(&db)
			};
			let true_area = {
				use geo::GeodesicArea;
				square(-0.13, 51.50, 0.01).geodesic_area_unsigned()
			};
			assert_eq!(area(true_area - 1.0).await.unwrap(), 1);
			assert_eq!(area(true_area + 1.0).await.unwrap(), 0);

			let path = line_string![(x: -0.1, y: 51.5), (x: -0.2, y: 51.6)];
			let true_length = {
				use geo::Length;
				Geodesic.length(&path)
			};
			NewRoute {
				path: Geography(path.clone()),
				raw: Wkb(Geography(path)),
				zone: None,
				bounds: None,
				collection: None,
			}
			.insert(&db)
			.await
			.unwrap();

			for (bound, count) in [(true_length - 0.01, 1), (true_length + 0.01, 0)] {
				let found = Route::query()
					.filter(|route| route.path.length().gt(bound))
					.count(&db)
					.await
					.unwrap();
				assert_eq!(found, count, "length past {bound} on {}", db.dialect());
			}

			// Covers counts the boundary on the globe.
			let inside = Place::query()
				.filter(|p| p.area.contains(&Geography(point!(x: -0.125, y: 51.505))))
				.count(&db)
				.await
				.unwrap();
			assert_eq!(inside, 1);
		}
	})
	.await;
}

#[tokio::test]
async fn nearest_orders_planar_shapes_and_pages() {
	on_both(|db| {
		async move {
			for (name, shape) in plots() {
				NewPlot {
					name: name.into(),
					shape: grid(shape.clone()),
					indexed: grid(shape),
				}
				.insert(&db)
				.await
				.unwrap();
			}

			let from = grid(point!(x: 500.0, y: 50.0));
			let expected = ["pond", "road", "shed", "field", "well", "barn", "tower"];

			for indexed in [false, true] {
				let rows = Plot::query()
					.nearest(|plots| column(plots, indexed), &from)
					.with_distance()
					.limit(10)
					.all(&db)
					.await
					.unwrap();
				let order: Vec<&str> = rows.iter().map(|(plot, _)| plot.name.as_str()).collect();
				assert_eq!(order, expected, "{} indexed: {indexed}", db.dialect());
				assert!((rows[0].1 - 100.0).abs() < 1e-9);

				let page = Plot::query()
					.nearest(|plots| column(plots, indexed), &from)
					.offset(2)
					.limit(2)
					.all(&db)
					.await
					.unwrap();
				assert_eq!(
					page.iter()
						.map(|plot| plot.name.as_str())
						.collect::<Vec<_>>(),
					["shed", "field"]
				);

				let filtered = Plot::query()
					.filter(|plots| plots.name.ne("pond"))
					.nearest(|plots| column(plots, indexed), &from)
					.first(&db)
					.await
					.unwrap()
					.unwrap();
				assert_eq!(filtered.name, "road");
			}

			// No limit orders everything; nothing to find finds nothing.
			assert_eq!(
				Plot::query()
					.nearest(|plots| plots.indexed, &from)
					.all(&db)
					.await
					.unwrap()
					.len(),
				7
			);
			assert!(
				Plot::query()
					.filter(|plots| plots.name.eq("nothing"))
					.nearest(|plots| plots.indexed, &from)
					.limit(3)
					.all(&db)
					.await
					.unwrap()
					.is_empty()
			);
		}
	})
	.await;
}

/// The keys in a spatial index's rows table, and how many boxes its
/// R*Tree holds, on SQLite.
/// What a live query reads through a tracked connection: the nearest rows
/// and spatial conditions are recorded on their table, and only the
/// database can tell whether a row is near or inside a shape.
#[tokio::test]
async fn nearest_and_spatial_reads_are_recorded() {
	on_both(async |db| {
		let ada = owner(&db, "Ada").await;
		let gallery = place(&db, ada, "Gallery", trafalgar()).await;
		let tracked = Tracked::new(db.with_context(Context::new().with(Viewer(ada))));

		let nearest = Place::query()
			.nearest(|p| p.location, &trafalgar())
			.limit(1)
			.all(&tracked)
			.await
			.unwrap();
		assert_eq!(nearest.len(), 1);
		let every = Place::query()
			.nearest(|p| p.location, &trafalgar())
			.with_distance()
			.all(&tracked)
			.await
			.unwrap();
		assert_eq!(every.len(), 1);
		let close = Place::query()
			.filter(|p| p.location.within_distance(&trafalgar(), Meters(100.0)))
			.count(&tracked)
			.await
			.unwrap();
		assert_eq!(close, 1);

		let reads = tracked.into_reads();
		let places: Vec<&ReadEntry> = reads
			.iter()
			.filter(|entry| entry.table == Place::table_name())
			.collect();
		assert_eq!(
			places.iter().map(|entry| entry.kind).collect::<Vec<_>>(),
			[ReadKind::Rows, ReadKind::Rows, ReadKind::Count]
		);
		// Distance orders the rows, so no column does.
		assert_eq!(places[0].order, Vec::new());
		assert_eq!(places[0].limit, Some(1));
		assert_eq!(
			places[1].keys,
			Some(vec![vec![Value::Uuid(gallery.id.uuid())]])
		);

		let row: BTreeMap<String, Value> = Place::COLUMNS
			.iter()
			.map(ToString::to_string)
			.zip(gallery.to_values().unwrap())
			.collect();

		for entry in &places {
			assert_eq!(entry.condition.eval(&row), Tri::Unknown, "{entry:?}");
		}

		let mut gone = row.clone();
		gone.insert("deleted_at".into(), Value::Timestamp(leptoskit_db::now()));

		for entry in &places {
			assert_eq!(entry.condition.eval(&gone), Tri::False, "{entry:?}");
		}
	})
	.await;
}

async fn index_contents(db: &Db, table: &str, column: &str) -> (i64, i64) {
	let count = |sql: String| {
		async move {
			match db
				.fetch(Statement {
					sql,
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
		count(format!(
			"SELECT count(*) FROM \"rtree_{table}_{column}_rows\""
		))
		.await,
		count(format!("SELECT count(*) FROM \"rtree_{table}_{column}\"")).await,
	)
}

#[tokio::test]
async fn every_write_keeps_sqlites_index_current() {
	let db = Db::connect("sqlite::memory:").await.unwrap();
	create_schema(&db).await;
	let ada = owner(&db, "ada").await;
	let mut gallery = place(&db, ada, "gallery", trafalgar()).await;
	assert_eq!(index_contents(&db, "places", "location").await, (1, 1));
	// A row without a shape is numbered but has no box.
	assert_eq!(index_contents(&db, "places", "area").await, (1, 0));

	let far = Geography(point!(x: 10.0, y: 10.0));
	let at_far = |db: &Db| {
		let db = db.clone();
		async move {
			Place::query()
				.filter(|p| {
					p.location
						.within_distance(&Geography(point!(x: 10.0, y: 10.0)), Meters(1.0))
				})
				.count(&db)
				.await
				.unwrap()
		}
	};
	gallery.location = far.clone();
	gallery.save(&db).await.unwrap();
	assert_eq!(at_far(&db).await, 1);
	assert_eq!(index_contents(&db, "places", "location").await, (1, 1));

	// Raw SQL writes are indexed too.
	let bytes = leptoskit_db::geo::encode_gpkg(&Geometry::Point(trafalgar().0), 4326).unwrap();
	db.execute(Statement {
		sql: "UPDATE \"places\" SET \"location\" = ?1".into(),
		args: vec![Value::Bytes(bytes)],
	})
	.await
	.unwrap();
	assert_eq!(at_far(&db).await, 0);
	assert_eq!(
		Place::query()
			.filter(|p| p.location.within_distance(&trafalgar(), Meters(1.0)))
			.count(&db)
			.await
			.unwrap(),
		1
	);

	// Bulk updates, soft deletes, and destroys.
	Place::update()
		.set(|p| p.location, far)
		.run(&db)
		.await
		.unwrap();
	assert_eq!(at_far(&db).await, 1);
	gallery = Place::get(&db, gallery.id).await.unwrap();
	gallery.delete(&db).await.unwrap();
	assert_eq!(index_contents(&db, "places", "location").await, (1, 1));
	Place::query().destroy(&db).await.unwrap();
	assert_eq!(index_contents(&db, "places", "location").await, (0, 0));

	// Composite keys in a namespace, and key changes.
	NewPin {
		board: 1,
		slot: 1,
		at: trafalgar(),
	}
	.insert(&db)
	.await
	.unwrap();
	db.execute(Statement {
		sql: "UPDATE \"maps_pins\" SET \"slot\" = 2".into(),
		args: Vec::new(),
	})
	.await
	.unwrap();
	let pins = Pin::query()
		.filter(|pin| pin.at.within_distance(&trafalgar(), Meters(1.0)))
		.all(&db)
		.await
		.unwrap();
	assert_eq!(pins.iter().map(|pin| pin.slot).collect::<Vec<_>>(), [2]);
	assert_eq!(index_contents(&db, "maps_pins", "at").await, (1, 1));
}

#[tokio::test]
async fn coordinates_and_validity_are_checked_everywhere() {
	on_both(|db| {
		async move {
			let ada = owner(&db, "ada").await;
			// `geo(…)` columns are checked by `validate()` before SQL runs.
			let out_of_range = NewPlace {
				owner_id: ada,
				name: "nowhere".into(),
				public: true,
				location: Geography(point!(x: 190.0, y: 0.0)),
				area: None,
				spot: None,
			}
			.insert(&db)
			.await;
			let Err(DbError::Invalid(errors)) = out_of_range else {
				panic!("{out_of_range:?}");
			};
			assert_eq!(errors.errors[0].field, "location");
			assert!(errors.to_string().contains("longitude 190"));

			// Every other column refuses to encode it.
			let path = |end: f64| line_string![(x: 0.0, y: 0.0), (x: end, y: 0.0)];

			for end in [181.0, f64::NAN, f64::INFINITY] {
				let refused = NewRoute {
					path: Geography(path(end)),
					raw: Wkb(Geography(path(1.0))),
					zone: None,
					bounds: None,
					collection: None,
				}
				.insert(&db)
				.await;
				assert!(matches!(refused, Err(DbError::Encode(_))), "{refused:?}");
			}

			// So do filters given such a shape.
			let filter = Place::query()
				.filter(|p| p.location.intersects(&Geography(point!(x: 0.0, y: 91.0))))
				.all(&db)
				.await;
			assert!(matches!(filter, Err(DbError::Encode(_))));

			// A bow tie crosses itself: `validate()` refuses it before SQL
			// runs.
			let bow_tie = polygon![
				(x: 0.0, y: 0.0),
				(x: 0.01, y: 0.01),
				(x: 0.01, y: 0.0),
				(x: 0.0, y: 0.01),
				(x: 0.0, y: 0.0),
			];
			let invalid = NewPlace {
				owner_id: ada,
				name: "bow tie".into(),
				public: true,
				location: trafalgar(),
				area: Some(Geography(MultiPolygon(vec![bow_tie.clone()]))),
				spot: None,
			}
			.insert(&db)
			.await;
			let Err(DbError::Invalid(errors)) = invalid else {
				panic!("{invalid:?}");
			};
			assert_eq!(errors.errors[0].field, "area");

			// The database refuses it too, from raw SQL.
			let gallery = place(&db, ada, "gallery", trafalgar()).await;

			let bytes = match db.dialect() {
				Dialect::Sqlite => {
					leptoskit_db::geo::encode_gpkg(
						&Geometry::MultiPolygon(MultiPolygon(vec![bow_tie])),
						4326,
					)
					.unwrap()
				}
				Dialect::Postgres => {
					leptoskit_db::geo::encode_ewkb(
						&Geometry::MultiPolygon(MultiPolygon(vec![bow_tie])),
						4326,
					)
					.unwrap()
				}
			};

			let sql = match db.dialect() {
				Dialect::Sqlite => "UPDATE \"places\" SET \"area\" = ?1",
				Dialect::Postgres => {
					"UPDATE \"places\" SET \"area\" = ST_GeomFromEWKB($1)::geography"
				}
			};

			let refused = db
				.execute(Statement {
					sql: sql.into(),
					args: vec![Value::Bytes(bytes)],
				})
				.await;
			assert!(matches!(refused, Err(DbError::Check { .. })), "{refused:?}");
			assert_eq!(Place::get(&db, gallery.id).await.unwrap().area, None);
		}
	})
	.await;
}

#[tokio::test]
async fn sqlite_refuses_blobs_that_arent_the_columns_shape() {
	let db = Db::connect("sqlite::memory:").await.unwrap();
	create_schema(&db).await;
	let ada = owner(&db, "ada").await;
	let gallery = place(&db, ada, "gallery", trafalgar()).await;
	let write = |bytes: Vec<u8>| {
		db.execute(Statement {
			sql: "UPDATE \"places\" SET \"location\" = ?1".into(),
			args: vec![Value::Bytes(bytes)],
		})
	};

	for bytes in [
		// A polygon in a point column.
		leptoskit_db::geo::encode_gpkg(&Geometry::Polygon(square(0.0, 0.0, 1.0)), 4326).unwrap(),
		// A point in the wrong coordinate system.
		leptoskit_db::geo::encode_gpkg(&Geometry::Point(point!(x: 0.0, y: 0.0)), 27700).unwrap(),
		// Out of range for geography.
		leptoskit_db::geo::encode_gpkg(&Geometry::Point(point!(x: 0.0, y: 95.0)), 4326).unwrap(),
		// Not a GeoPackage geometry at all.
		vec![1, 2, 3],
	] {
		assert!(matches!(write(bytes).await, Err(DbError::Check { .. })));
	}

	assert_eq!(
		Place::get(&db, gallery.id).await.unwrap().location,
		trafalgar()
	);

	// Reading a shape in the wrong coordinate system fails rather than
	// guessing.
	let decoded = <Geo<Point, Grid> as leptoskit_db::Column>::from_value(Value::Bytes(
		leptoskit_db::geo::encode_gpkg(&Geometry::Point(point!(x: 0.0, y: 0.0)), 4326).unwrap(),
	));
	assert!(decoded.unwrap_err().to_string().contains("SRID 4326"));
}

#[tokio::test]
async fn postgis_operations_run_on_postgres_and_fail_on_sqlite() {
	let sqlite = Db::connect("sqlite::memory:").await.unwrap();
	create_schema(&sqlite).await;
	let refused = Plot::query()
		.filter(|plots| {
			plots
				.shape
				.buffer(10.0)
				.intersects(&grid(point!(x: 0.0, y: 0.0)))
		})
		.all(&sqlite)
		.await;
	assert!(
		matches!(&refused, Err(DbError::Encode(error)) if error.to_string().contains("PostGIS")),
		"{:?}",
		refused.err()
	);

	let Some(cluster) = support::postgres::PostgresCluster::start_with("postgis") else {
		return;
	};
	let db = Db::connect(&cluster.database()).await.unwrap();
	create_schema(&db).await;

	for (name, shape) in plots() {
		NewPlot {
			name: name.into(),
			shape: grid(shape.clone()),
			indexed: grid(shape),
		}
		.insert(&db)
		.await
		.unwrap();
	}

	let found = |condition: fn(&PlotColumns) -> Expr| {
		let db = db.clone();
		async move {
			let rows = Plot::query().filter(condition).all(&db).await.unwrap();
			names(&rows, |plot| &plot.name)
		}
	};

	// Grown by 10 m, the field and the well on its edge both reach 5 m
	// past it.
	assert_eq!(
		found(|plots| {
			plots
				.shape
				.buffer(10.0)
				.contains(&grid(point!(x: 50.0, y: 105.0)))
		})
		.await,
		["field", "well"]
	);
	// The tower, in Web Mercator, is somewhere near Null Island's west.
	assert_eq!(
		found(|plots| {
			plots
				.shape
				.transform::<Srid<3857>>()
				.within(&Geo::<Rect, Srid<3857>>::new(Rect::new(
					coord! { x: -1_000_000.0, y: 5_000_000.0 },
					coord! { x: 0.0, y: 7_000_000.0 },
				)))
		})
		.await
		.len(),
		7
	);
	assert_eq!(
		found(|plots| {
			plots
				.shape
				.term()
				.simplify(1.0)
				.union(&grid(square(1_000.0, 1_000.0, 10.0)))
				.area()
				.gt(10_099.0)
		})
		.await,
		["field"]
	);
}

#[tokio::test]
async fn audit_and_raw_reads_see_the_same_shape_on_both() {
	on_both(|db| {
		async move {
			let ada = owner(&db, "ada").await;
			place(&db, ada, "gallery", trafalgar()).await;
			let entries = leptoskit_db::AuditEntry::for_table::<Place>()
				.all(&db.system("reading the log"))
				.await
				.unwrap();
			assert_eq!(entries.len(), 1);
			let after = entries[0].after.as_ref().unwrap().0.clone();
			let location = after["location"].clone();
			assert!(location.is_string(), "{location}");
		}
	})
	.await;
}

// The property: what each viewer finds with spatial filters and nearest,
// against a plain-Rust model of the policy and the measurements.
const OWNERS: usize = 3;

#[derive(Clone, Debug)]
struct Placed {
	owner: usize,
	public: bool,
	deleted: bool,
	/// Longitude and latitude, in millionths of a degree around London.
	at: (i32, i32),
	/// A grid position, in whole metres.
	spot: Option<(i32, i32)>,
}

#[derive(Clone, Debug)]
enum Ask {
	Near { center: (i32, i32), meters: f64 },
	InBox { min: (i32, i32), max: (i32, i32) },
	Nearest { center: (i32, i32), limit: u64 },
}

#[derive(Clone, Debug)]
struct Spec {
	places: Vec<Placed>,
	viewer: Option<usize>,
	ask: Ask,
}

fn lon_lat((x, y): (i32, i32)) -> Point {
	point!(x: -0.1 + f64::from(x) / 1e6, y: 51.5 + f64::from(y) / 1e6)
}

fn spec() -> impl Strategy<Value = Spec> {
	let placed = (
		0..OWNERS,
		any::<bool>(),
		prop::bool::weighted(0.15),
		(-20_000..20_000, -20_000..20_000),
		prop::option::of((0..200, 0..200)),
	)
		.prop_map(|(owner, public, deleted, at, spot)| {
			Placed {
				owner,
				public,
				deleted,
				at,
				spot,
			}
		});
	let ask = prop_oneof![
		((-20_000..20_000, -20_000..20_000), 0.0..2_500.0)
			.prop_map(|(center, meters)| Ask::Near { center, meters }),
		((0..200, 0..200), (0..100, 0..100)).prop_map(|(min, size)| {
			Ask::InBox {
				min,
				max: (min.0 + size.0, min.1 + size.1),
			}
		}),
		((-20_000..20_000, -20_000..20_000), 1..6_u64)
			.prop_map(|(center, limit)| Ask::Nearest { center, limit }),
	];

	(
		prop::collection::vec(placed, 0..14),
		prop::option::of(0..OWNERS),
		ask,
	)
		.prop_map(|(mut places, viewer, ask)| {
			// Distinct positions, so the nearest order has no ties.
			let mut seen = std::collections::HashSet::new();
			places.retain(|place| seen.insert(place.at));
			Spec {
				places,
				viewer,
				ask,
			}
		})
}

impl Spec {
	fn visible(&self, place: &Placed) -> bool {
		!place.deleted && (place.public || self.viewer == Some(place.owner))
	}
}

async fn agrees_with_the_model(db: &Db, spec: Spec) -> Result<(), TestCaseError> {
	let system = db.system("resetting the property");
	Place::query().destroy(&system).await.unwrap();
	Owner::query().destroy(&system).await.unwrap();
	let mut owners = Vec::new();

	for index in 0..OWNERS {
		owners.push(owner(&system, &format!("o{index}")).await);
	}

	let mut stored = Vec::new();

	for (index, placed) in spec.places.iter().enumerate() {
		let mut row = NewPlace {
			owner_id: owners[placed.owner],
			name: format!("p{index}"),
			public: placed.public,
			location: Geography(lon_lat(placed.at)),
			area: None,
			spot: placed
				.spot
				.map(|(x, y)| Geo::new(point!(x: f64::from(x), y: f64::from(y)))),
		}
		.insert(&system)
		.await
		.unwrap();

		if placed.deleted {
			row.delete(&system).await.unwrap();
		}

		stored.push((row.id, placed));
	}

	let request = match spec.viewer {
		Some(viewer) => db.with_context(Context::new().with(Viewer(owners[viewer]))),
		None => db.with_context(Context::new()),
	};

	let visible = stored.iter().filter(|(_, placed)| spec.visible(placed));

	match &spec.ask {
		Ask::Near { center, meters } => {
			let center = lon_lat(*center);
			let mut expected: Vec<Id<Place>> = visible
				.filter(|(_, placed)| Geodesic.distance(center, lon_lat(placed.at)) <= *meters)
				.map(|(id, _)| *id)
				.collect();
			let mut found: Vec<Id<Place>> = Place::query()
				.filter(|p| {
					p.location
						.within_distance(&Geography(center), Meters(*meters))
				})
				.all(&request)
				.await
				.unwrap()
				.into_iter()
				.map(|place| place.id)
				.collect();
			found.sort();
			expected.sort();
			prop_assert_eq!(found, expected);
		}
		Ask::InBox { min, max } => {
			let area = Rect::new(
				coord! { x: f64::from(min.0) + 0.5, y: f64::from(min.1) + 0.5 },
				coord! { x: f64::from(max.0) + 0.5, y: f64::from(max.1) + 0.5 },
			);
			let mut expected: Vec<Id<Place>> = visible
				.filter(|(_, placed)| {
					placed.spot.is_some_and(|(x, y)| {
						point!(x: f64::from(x), y: f64::from(y))
							.relate(&area.to_polygon())
							.is_intersects()
					})
				})
				.map(|(id, _)| *id)
				.collect();
			let mut found: Vec<Id<Place>> = Place::query()
				.filter(|p| p.spot.intersects(&Geo::<Rect, Grid>::new(area)))
				.all(&request)
				.await
				.unwrap()
				.into_iter()
				.map(|place| place.id)
				.collect();
			found.sort();
			expected.sort();
			prop_assert_eq!(found, expected);
		}
		Ask::Nearest { center, limit } => {
			let center = lon_lat(*center);
			let mut expected: Vec<(f64, Id<Place>)> = visible
				.map(|(id, placed)| (Geodesic.distance(center, lon_lat(placed.at)), *id))
				.collect();
			expected.sort_by(|left, right| left.partial_cmp(right).unwrap());
			expected.truncate(*limit as usize);
			let found = Place::query()
				.nearest(|p| p.location, &Geography(center))
				.with_distance()
				.limit(*limit)
				.all(&request)
				.await
				.unwrap();
			let ids: Vec<Id<Place>> = found.iter().map(|(place, _)| place.id).collect();
			prop_assert_eq!(ids, expected.iter().map(|(_, id)| *id).collect::<Vec<_>>());

			for ((_, distance), (model, _)) in found.iter().zip(&expected) {
				prop_assert!((distance.0 - model).abs() < 1e-6);
			}
		}
	}

	Ok(())
}

fn run_property(db: &Db, runtime: &tokio::runtime::Runtime, cases: u32) {
	let mut runner = TestRunner::new(Config {
		cases,
		failure_persistence: None,
		..Config::default()
	});
	let outcome = runner.run(&spec(), |spec| {
		runtime.block_on(agrees_with_the_model(db, spec))
	});

	if let Err(error) = outcome {
		panic!("{error}");
	}
}

#[test]
fn spatial_queries_agree_with_the_model_on_sqlite() {
	let runtime = tokio::runtime::Runtime::new().unwrap();
	let db = runtime.block_on(async {
		let db = Db::connect("sqlite::memory:").await.unwrap();
		create_schema(&db).await;
		db
	});
	run_property(&db, &runtime, 96);
}

#[test]
fn spatial_queries_agree_with_the_model_on_postgres() {
	let Some(cluster) = support::postgres::PostgresCluster::start_with("postgis") else {
		return;
	};
	let runtime = tokio::runtime::Runtime::new().unwrap();
	let db = runtime.block_on(async {
		let db = Db::connect(&cluster.database()).await.unwrap();
		create_schema(&db).await;
		db
	});
	run_property(&db, &runtime, 48);
}

#[tokio::test]
async fn nearest_takes_filters_orderings_and_deleted_rows() {
	on_both(|db| {
		async move {
			let ada = owner(&db, "ada").await;
			let mut gallery = place(&db, ada, "gallery", trafalgar()).await;
			// Two places at the same spot: the name breaks the tie.
			place(&db, ada, "b", Geography(point!(x: -0.1270, y: 51.5194))).await;
			place(&db, ada, "a", Geography(point!(x: -0.1270, y: 51.5194))).await;
			gallery.delete(&db).await.unwrap();

			let names = |rows: Vec<Place>| rows.into_iter().map(|p| p.name).collect::<Vec<_>>();
			let found = Place::query()
				.nearest(|p| p.location, &trafalgar())
				.filter(|p| p.name.ne("nothing"))
				.order_by(|p| p.name.desc())
				.limit(5)
				.all(&db)
				.await
				.unwrap();
			assert_eq!(names(found), ["b", "a"]);

			let with_deleted = Place::query()
				.nearest(|p| p.location, &trafalgar())
				.with_deleted()
				.limit(5)
				.all(&db)
				.await
				.unwrap();
			assert_eq!(with_deleted[0].name, "gallery");

			let (second, distance) = Place::query()
				.nearest(|p| p.location, &trafalgar())
				.order_by(|p| p.name.asc())
				.with_distance()
				.offset(1)
				.first(&db)
				.await
				.unwrap()
				.unwrap();
			assert_eq!(second.name, "b");
			assert!(distance.0 > 1_000.0);
			assert!(
				Place::query()
					.filter(|p| p.name.eq("nothing"))
					.nearest(|p| p.location, &trafalgar())
					.with_distance()
					.limit(1)
					.all(&db)
					.await
					.unwrap()
					.is_empty()
			);
		}
	})
	.await;
}
