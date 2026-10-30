---
"release": minor
---

# Geospatial columns on PostGIS and SQLite

The `db-geo` feature adds `leptoskit::db::geo`: `Geography<Point>` for places on the globe, measured in metres, and `Geo<Polygon, Srid<27700>>` for planar shapes in any coordinate system, where mixing systems fails to compile. Any `geo-types` shape fits, and every shape travels as GeoJSON, or as base64 WKB inside `Wkb<…>`. `#[db(geo(index))]` adds a spatial index and `#[db(geo(valid))]` refuses invalid shapes in Rust and in the database.

Spatial filters work like any other filter, with the table's policies: `intersects`, `contains`, `within`, `disjoint`, `touches`, `within_distance`, and `distance_to`, `area`, and `length` to compare. `query().nearest(|p| p.location, &here).with_distance().limit(20)` returns the nearest rows first. Postgres uses PostGIS, created by the first migration that needs it, with GiST indexes, and adds `buffer`, `simplify`, `transform`, and `union`. SQLite uses a built-in layer with nothing to install: GeoPackage-encoded columns checked by the database, an R*Tree per index kept current by triggers, and functions registered on every connection, measuring the globe with the same geodesic algorithm as PostGIS. `[db.geo] geopackage = true` makes a SQLite file open as layers in QGIS and GDAL.
