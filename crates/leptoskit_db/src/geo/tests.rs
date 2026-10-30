use super::*;

fn grid<T>(shape: T) -> Geo<T, Srid<27700>> {
	Geo::new(shape)
}

#[test]
fn geography_travels_as_geojson() {
	let here = Geography(point!(x: -0.1276, y: 51.5072));
	let json = serde_json::to_value(&here).unwrap();
	assert_eq!(
		json,
		serde_json::json!({ "type": "Point", "coordinates": [-0.1276, 51.5072] })
	);
	assert_eq!(
		serde_json::from_value::<Geography<Point>>(json).unwrap(),
		here
	);

	let area = Geography(polygon![
		(x: 0.0, y: 0.0),
		(x: 1.0, y: 0.0),
		(x: 1.0, y: 1.0),
		(x: 0.0, y: 0.0),
	]);
	let text = serde_json::to_string(&area).unwrap();
	assert_eq!(
		text,
		r#"{"type":"Polygon","coordinates":[[[0.0,0.0],[1.0,0.0],[1.0,1.0],[0.0,0.0]]]}"#
	);
	assert_eq!(
		serde_json::from_str::<Geography<Polygon>>(&text).unwrap(),
		area
	);
	assert_eq!(
		serde_json::from_str::<Geography<Geometry>>(&text).unwrap(),
		Geography(Geometry::Polygon(area.0))
	);
}

#[test]
fn deserializing_refuses_the_wrong_shape_and_bad_coordinates() {
	let refuse = |json: serde_json::Value| {
		serde_json::from_value::<Geography<Point>>(json)
			.unwrap_err()
			.to_string()
	};

	assert!(
		refuse(serde_json::json!({ "type": "Point", "coordinates": [181.0, 0.0] }))
			.contains("longitude 181")
	);
	assert!(
		refuse(serde_json::json!({ "type": "Point", "coordinates": [0.0, -91.0] }))
			.contains("latitude -91")
	);
	assert!(
		refuse(
			serde_json::json!({ "type": "LineString", "coordinates": [[0.0, 0.0], [1.0, 1.0]] })
		)
		.contains("expected a Point, found a LineString")
	);
	assert!(serde_json::from_value::<Geography<Point>>(serde_json::json!("POINT(0 0)")).is_err());

	// Planar systems have no range.
	let far: Geo<Point, Srid<27700>> = serde_json::from_value(
		serde_json::json!({ "type": "Point", "coordinates": [530_000.0, 180_000.0] }),
	)
	.unwrap();
	assert_eq!(far, grid(point!(x: 530_000.0, y: 180_000.0)));
	assert!(format!("{far:?}").contains("27700"));
	assert_eq!(far.clone().into_inner(), point!(x: 530_000.0, y: 180_000.0));
}

#[test]
fn rectangles_are_polygons_with_four_square_corners() {
	let rect = Rect::new(coord! { x: 1.0, y: 2.0 }, coord! { x: 3.0, y: 5.0 });
	let json = serde_json::to_value(grid(rect)).unwrap();
	assert_eq!(json["type"], "Polygon");
	assert_eq!(
		serde_json::from_value::<Geo<Rect, Srid<27700>>>(json).unwrap(),
		grid(rect)
	);

	for not_a_rect in [
		polygon![(x: 0.0, y: 0.0), (x: 2.0, y: 0.0), (x: 1.0, y: 1.0), (x: 0.0, y: 0.0)],
		polygon![
			(x: 0.0, y: 0.0),
			(x: 2.0, y: 0.0),
			(x: 2.0, y: 2.0),
			(x: 1.0, y: 2.0),
			(x: 0.0, y: 0.0),
		],
		Polygon::new(
			rect.to_polygon().exterior().clone(),
			vec![
				Rect::new(coord! { x: 1.5, y: 3.0 }, coord! { x: 2.0, y: 4.0 })
					.to_polygon()
					.exterior()
					.clone(),
			],
		),
	] {
		assert!(Rect::from_geometry(Geometry::Polygon(not_a_rect)).is_err());
	}

	assert!(Rect::from_geometry(Geometry::Point(point!(x: 0.0, y: 0.0))).is_err());
	assert_eq!(grid(rect).bounding_box(), Some(rect));
}

#[test]
fn any_shape_accepts_lines_rects_and_triangles_as_stored_shapes() {
	let line = geo_types::Line::new(coord! { x: 0.0, y: 0.0 }, coord! { x: 1.0, y: 1.0 });
	assert_eq!(
		Geometry::from_geometry(Geometry::Line(line)).unwrap(),
		Geometry::LineString(line.into())
	);
	let rect = Rect::new(coord! { x: 0.0, y: 0.0 }, coord! { x: 1.0, y: 1.0 });
	assert_eq!(
		Geometry::from_geometry(Geometry::Rect(rect)).unwrap(),
		Geometry::Polygon(rect.to_polygon())
	);
	let triangle = geo_types::Triangle::new(
		coord! { x: 0.0, y: 0.0 },
		coord! { x: 1.0, y: 0.0 },
		coord! { x: 0.0, y: 1.0 },
	);
	assert_eq!(
		Geometry::from_geometry(Geometry::Triangle(triangle)).unwrap(),
		Geometry::Polygon(triangle.to_polygon())
	);
}

#[test]
fn coordinates_are_visited_in_every_shape() {
	let collection = Geometry::GeometryCollection(GeometryCollection(vec![
		Geometry::Line(geo_types::Line::new(
			coord! { x: 0.0, y: 0.0 },
			coord! { x: 1.0, y: 1.0 },
		)),
		Geometry::MultiPoint(vec![point!(x: 2.0, y: 2.0)].into()),
		Geometry::MultiLineString(MultiLineString::new(vec![
			line_string![(x: 3.0, y: 3.0), (x: 4.0, y: 4.0)],
		])),
		Geometry::MultiPolygon(
			vec![Polygon::new(
				line_string![(x: 0.0, y: 0.0), (x: 9.0, y: 0.0), (x: 9.0, y: 9.0), (x: 0.0, y: 0.0)],
				vec![
					line_string![(x: 1.0, y: 1.0), (x: 2.0, y: 1.0), (x: 2.0, y: 2.0), (x: 1.0, y: 1.0)],
				],
			)]
			.into(),
		),
		Geometry::Rect(Rect::new(
			coord! { x: -1.0, y: -1.0 },
			coord! { x: 0.0, y: 0.0 },
		)),
		Geometry::Triangle(geo_types::Triangle::new(
			coord! { x: 0.0, y: 0.0 },
			coord! { x: 1.0, y: 0.0 },
			coord! { x: 0.0, y: 10.0 },
		)),
	]));
	let mut count = 0;
	visit_coords(&collection, &mut |_| count += 1);
	assert_eq!(count, 2 + 1 + 2 + 8 + 5 + 4);
	assert_eq!(envelope(&collection), Some([-1.0, 9.0, -1.0, 10.0]));
	assert!(check_coordinates(&collection, true).is_ok());
	assert_eq!(
		envelope(&Geometry::GeometryCollection(
			GeometryCollection(Vec::new())
		)),
		None
	);
}

#[test]
fn json_schemas_describe_geojson() {
	let point = schemars::schema_for!(Geography<Point>);
	let point = serde_json::to_value(point).unwrap();
	assert_eq!(point["properties"]["type"]["const"], "Point");
	assert_eq!(
		point["properties"]["coordinates"]["prefixItems"][0]["maximum"],
		180.0
	);
	assert_eq!(Geography::<Point>::schema_name(), "GeoJsonPointGeography");

	let grid_polygon =
		serde_json::to_value(schemars::schema_for!(Geo<MultiPolygon, Srid<27700>>)).unwrap();
	assert_eq!(grid_polygon["title"], "GeoJSON MultiPolygon in EPSG:27700");
	assert!(
		grid_polygon["properties"]["coordinates"]["items"]["items"]["items"]["prefixItems"][0]
			.get("maximum")
			.is_none()
	);

	for name in [
		"LineString",
		"MultiPoint",
		"Polygon",
		"MultiLineString",
		"GeometryCollection",
	] {
		let schema = geometry_schema(name, false);
		assert_eq!(schema["properties"]["type"]["const"], name);
	}

	let any = serde_json::to_value(schemars::schema_for!(Geography<Geometry>)).unwrap();
	assert_eq!(any["oneOf"].as_array().unwrap().len(), 7);
	let wkb = serde_json::to_value(schemars::schema_for!(Wkb<Geography<Point>>)).unwrap();
	assert_eq!(wkb["contentEncoding"], "base64");
	assert_eq!(Wkb::<Geography<Point>>::schema_name(), "Wkb");
	assert_eq!(
		Geo::<Point, Srid<3857>>::schema_name(),
		"GeoJsonPointSrid3857"
	);
}

#[test]
fn wkb_travels_as_base64() {
	let path = Wkb(Geography(line_string![(x: 0.0, y: 0.0), (x: 1.0, y: 2.0)]));
	let json = serde_json::to_value(&path).unwrap();
	let text = json.as_str().unwrap();
	let bytes = {
		use base64::Engine;
		base64::engine::general_purpose::STANDARD
			.decode(text)
			.unwrap()
	};
	assert_eq!(bytes[0], 1, "little-endian");
	assert_eq!(&bytes[1..5], &2_u32.to_le_bytes(), "a line string");
	assert_eq!(
		serde_json::from_value::<Wkb<Geography<LineString>>>(json).unwrap(),
		path
	);

	assert!(serde_json::from_value::<Wkb<Geography<LineString>>>(serde_json::json!("!!")).is_err());
	let point = {
		use base64::Engine;
		base64::engine::general_purpose::STANDARD
			.encode(codec::encode_wkb(&Geometry::Point(point!(x: 500.0, y: 0.0))).unwrap())
	};
	assert!(serde_json::from_value::<Wkb<Geography<Point>>>(serde_json::json!(point)).is_err());
	assert!(
		serde_json::from_value::<Wkb<Geo<Point, Srid<27700>>>>(serde_json::json!(point)).is_ok()
	);
}

#[test]
fn ewkb_and_geopackage_round_trip_with_their_srid() {
	let shape = Geometry::Polygon(polygon![
		(x: 1.0, y: 2.0),
		(x: 5.0, y: 2.0),
		(x: 5.0, y: 7.0),
		(x: 1.0, y: 2.0),
	]);
	let ewkb = encode_ewkb(&shape, 27700).unwrap();
	assert_eq!(decode_any(&ewkb).unwrap(), (27700, shape.clone()));
	assert_eq!(&ewkb[5..9], &27700_u32.to_le_bytes());

	let gpkg = encode_gpkg(&shape, 27700).unwrap();
	assert_eq!(&gpkg[..4], b"GP\x00\x03");
	assert_eq!(decode_any(&gpkg).unwrap(), (27700, shape.clone()));
	let header = codec::Gpkg::header(&gpkg).unwrap();
	assert_eq!(header.envelope, Some([1.0, 5.0, 2.0, 7.0]));
	assert!(!header.empty);

	// Plain WKB has no SRID.
	assert_eq!(
		decode_any(&codec::encode_wkb(&shape).unwrap()).unwrap().0,
		0
	);

	let empty = Geometry::MultiPolygon(MultiPolygon(Vec::new()));
	let gpkg = encode_gpkg(&empty, 4326).unwrap();
	let header = codec::Gpkg::header(&gpkg).unwrap();
	assert!(header.empty);
	assert_eq!(header.envelope, None);
	assert_eq!(decode_any(&gpkg).unwrap(), (4326, empty));
}

#[test]
fn big_endian_headers_and_other_envelopes_are_read() {
	let point = Geometry::Point(point!(x: 3.0, y: 4.0));
	let wkb = codec::encode_wkb(&point).unwrap();

	// A big-endian header with an XYZ envelope, as other writers may make.
	let mut bytes = b"GP\x00".to_vec();
	bytes.push(0b0000_0100);
	bytes.extend_from_slice(&3857_i32.to_be_bytes());
	for bound in [3.0_f64, 3.0, 4.0, 4.0, 0.0, 0.0] {
		bytes.extend_from_slice(&bound.to_be_bytes());
	}
	bytes.extend_from_slice(&wkb);
	let header = codec::Gpkg::header(&bytes).unwrap();
	assert_eq!(header.srid, 3857);
	assert_eq!(header.envelope, Some([3.0, 3.0, 4.0, 4.0]));
	assert_eq!(decode_any(&bytes).unwrap(), (3857, point.clone()));

	// Big-endian EWKB, as PostGIS can be asked for.
	let mut big = vec![0_u8];
	big.extend_from_slice(&(1_u32 | 0x2000_0000).to_be_bytes());
	big.extend_from_slice(&4326_u32.to_be_bytes());
	big.extend_from_slice(&3.0_f64.to_be_bytes());
	big.extend_from_slice(&4.0_f64.to_be_bytes());
	assert_eq!(decode_any(&big).unwrap(), (4326, point));
}

#[test]
fn malformed_bytes_are_refused() {
	let refuse = |bytes: &[u8]| decode_any(bytes).unwrap_err();

	assert!(refuse(&[]).contains("byte order"));
	assert!(refuse(&[7, 1, 0, 0, 0]).contains("byte order"));
	assert!(refuse(&[1, 1, 0, 0, 0x20]).contains("too short"));
	assert!(refuse(b"GP").contains("too short"));
	assert!(refuse(b"GP\x01\x03\x00\x00\x00\x00").contains("version 1"));
	assert!(refuse(b"GP\x00\x23\x00\x00\x00\x00").contains("extended"));
	assert!(refuse(b"GP\x00\x0b\x00\x00\x00\x00").contains("envelope kind 5"));
	assert!(refuse(b"GP\x00\x03\x00\x00\x00\x00").contains("too short"));
	assert!(codec::Gpkg::header(&[1, 2, 3]).unwrap_err().contains("GP"));

	// An empty point can't be held.
	let mut empty_point = vec![1_u8];
	empty_point.extend_from_slice(&1_u32.to_le_bytes());
	empty_point.extend_from_slice(&f64::NAN.to_le_bytes());
	empty_point.extend_from_slice(&f64::NAN.to_le_bytes());
	assert!(refuse(&empty_point).contains("empty point"));

	// Nor a point with a Z value.
	let mut z = vec![1_u8];
	z.extend_from_slice(&1001_u32.to_le_bytes());
	for value in [1.0_f64, 2.0, 3.0] {
		z.extend_from_slice(&value.to_le_bytes());
	}
	assert!(refuse(&z).contains("two-dimensional"));

	assert!(encode_gpkg(&Geometry::Point(point!(x: 0.0, y: 0.0)), u32::MAX).is_err());
}

#[test]
fn validation_checks_coordinates_and_validity() {
	use crate::ValidationErrors;

	let bow_tie = Geography(polygon![
		(x: 0.0, y: 0.0),
		(x: 1.0, y: 1.0),
		(x: 1.0, y: 0.0),
		(x: 0.0, y: 1.0),
		(x: 0.0, y: 0.0),
	]);
	let mut errors = ValidationErrors::default();
	rules::geo(&mut errors, "area", &bow_tie, true);
	rules::geo(&mut errors, "area", &bow_tie, false);
	rules::geo(
		&mut errors,
		"far",
		&Some(Geography(point!(x: 0.0, y: 100.0))),
		false,
	);
	rules::geo(&mut errors, "none", &Option::<Geography<Point>>::None, true);
	assert_eq!(errors.errors.len(), 2, "{errors}");
	assert_eq!(errors.errors[0].field, "area");
	assert!(errors.to_string().contains("valid shape"));
	assert!(errors.to_string().contains("latitude 100"));
}

#[test]
fn meters_convert_to_and_from_numbers() {
	assert_eq!(f64::from(Meters(2.5)), 2.5);
	assert_eq!(Meters::from(3.0), Meters(3.0));
	assert_eq!(serde_json::to_string(&Meters(1.5)).unwrap(), "1.5");
	assert_eq!(
		trafalgar().clone().into_inner(),
		point!(x: -0.1281, y: 51.5080)
	);
	assert_eq!(
		trafalgar().bounding_box().unwrap().min(),
		coord! { x: -0.1281, y: 51.5080 }
	);
}

fn trafalgar() -> Geography<Point> {
	Geography(point!(x: -0.1281, y: 51.5080))
}
