use super::*;

#[test]
fn float_vectors_travel_as_arrays_of_numbers() {
	let vector = Vector::new([0.5, -1.0, 2.0]);
	let json = serde_json::to_value(&vector).unwrap();
	assert_eq!(json, serde_json::json!([0.5, -1.0, 2.0]));
	assert_eq!(serde_json::from_value::<Vector<3>>(json).unwrap(), vector);

	let half = HalfVector::from(&vector);
	assert_eq!(
		serde_json::to_value(&half).unwrap(),
		serde_json::json!([0.5, -1.0, 2.0])
	);
	assert_eq!(
		serde_json::from_str::<HalfVector<3>>("[0.5, -1.0, 2.0]").unwrap(),
		half
	);
	assert_eq!(half.to_vector(), vector);
}

#[test]
fn float_vectors_refuse_the_wrong_length_and_values_that_arent_finite() {
	let refuse = |json: &str| {
		serde_json::from_str::<Vector<3>>(json)
			.unwrap_err()
			.to_string()
	};

	assert!(refuse("[1, 2]").contains("a Vector<3> has 3 values, not 2"));
	assert!(refuse("[1, 2, 3, 4]").contains("not 4"));
	assert!(
		serde_json::from_str::<HalfVector<1>>("[1e6]")
			.unwrap_err()
			.to_string()
			.contains("isn't finite")
	);
	assert_eq!(
		Vector::<2>::try_from(vec![1.0, 2.0, 3.0]).unwrap_err(),
		vec![1.0, 2.0, 3.0]
	);
	assert_eq!(
		Vector::<2>::try_from(vec![1.0, 2.0]).unwrap(),
		Vector::new([1.0, 2.0])
	);
}

#[test]
fn normalising_scales_to_length_one() {
	let unit = Vector::new([3.0, 4.0]).normalized().unwrap();
	assert_eq!(unit.as_slice(), &[0.6, 0.8]);
	assert_eq!(Vector::new([0.0, 0.0]).normalized(), None);
}

#[test]
fn bits_pack_lowest_first_and_travel_as_text() {
	let mut bits = [false; 16];
	bits[0] = true;
	bits[9] = true;
	let vector = BitVector::from_bits(bits);
	assert_eq!(vector.as_bytes(), &[0b0000_0001, 0b0000_0010]);
	assert_eq!(vector.get(9), Some(true));
	assert_eq!(vector.get(10), Some(false));
	assert_eq!(vector.get(16), None);

	assert_eq!(vector.count_ones(), 2);

	let json = serde_json::to_value(&vector).unwrap();
	assert_eq!(json, serde_json::json!("1000000001000000"));
	assert_eq!(
		serde_json::from_value::<BitVector<16>>(json).unwrap(),
		vector
	);
	assert_eq!(BitVector::<16>::from_bytes(&[1, 2]).unwrap(), vector);
	assert_eq!(BitVector::<16>::from_bytes(&[1]), None);

	for json in ["\"10\"", "\"10000000010000002\""] {
		assert!(
			serde_json::from_str::<BitVector<16>>(json).is_err(),
			"{json}"
		);
	}
}

#[test]
fn binary_quantisation_sets_the_positive_dimensions() {
	let vector = Vector::new([0.2, -0.1, 0.0, 3.0, -2.0, 1.0, 0.5, -0.5]);
	assert_eq!(BitVector::quantize(&vector).as_bytes(), &[0b0110_1001]);
}

#[test]
fn sparse_vectors_keep_their_nonzero_entries_in_order() {
	let vector = SparseVector::<10>::new([(7, 2.5), (1, -1.0), (4, 0.0)]).unwrap();
	assert_eq!(vector.nonzero(), 2);
	assert_eq!(vector.entries().collect::<Vec<_>>(), [(1, -1.0), (7, 2.5)]);
	assert_eq!(vector.get(7), 2.5);
	assert_eq!(vector.get(4), 0.0);

	let json = serde_json::to_value(&vector).unwrap();
	assert_eq!(
		json,
		serde_json::json!({ "indices": [1, 7], "values": [-1.0, 2.5] })
	);
	assert_eq!(
		serde_json::from_value::<SparseVector<10>>(json).unwrap(),
		vector
	);

	let mut dense = [0.0; 10];
	dense[1] = -1.0;
	dense[7] = 2.5;
	assert_eq!(SparseVector::from_dense(&dense).unwrap(), vector);

	assert!(
		SparseVector::<10>::new([(10, 1.0)])
			.unwrap_err()
			.contains("past a SparseVector<10>'s last, 9")
	);
	assert!(
		SparseVector::<10>::new([(2, 1.0), (2, 3.0)])
			.unwrap_err()
			.contains("appears twice")
	);
	assert!(serde_json::from_str::<SparseVector<10>>(r#"{"indices": [1], "values": []}"#).is_err());
}

#[test]
fn json_schemas_name_the_dimensions() {
	let mut generator = SchemaGenerator::default();
	let schema = serde_json::to_value(Vector::<3>::json_schema(&mut generator)).unwrap();
	assert_eq!(schema["minItems"], 3);
	assert_eq!(schema["maxItems"], 3);

	let schema = serde_json::to_value(BitVector::<16>::json_schema(&mut generator)).unwrap();
	assert_eq!(schema["pattern"], "^[01]{16}$");
	assert_eq!(Vector::<1536>::schema_name(), "Vector1536");
}

#[test]
fn every_kind_names_its_schema_and_shape() {
	let mut generator = SchemaGenerator::default();

	let schema = serde_json::to_value(HalfVector::<4>::json_schema(&mut generator)).unwrap();
	assert_eq!(schema["title"], "HalfVector<4>");
	assert_eq!(schema["maxItems"], 4);
	assert_eq!(HalfVector::<4>::schema_name(), "HalfVector4");
	assert_eq!(BitVector::<16>::schema_name(), "BitVector16");

	let schema = serde_json::to_value(SparseVector::<30>::json_schema(&mut generator)).unwrap();
	assert_eq!(schema["properties"]["indices"]["items"]["maximum"], 29);
	assert_eq!(schema["required"], serde_json::json!(["indices", "values"]));
	assert_eq!(SparseVector::<30>::schema_name(), "SparseVector30");
}

#[test]
fn debug_output_names_the_type_and_dimensions() {
	let vector = Vector::from([1.0, -2.5]);
	assert_eq!(vector.as_array(), &[1.0, -2.5]);
	assert_eq!(format!("{vector:?}"), "Vector<2>[1.0, -2.5]");

	let half = HalfVector::from(&vector);
	assert_eq!(half.as_slice(), &[f16::from_f32(1.0), f16::from_f32(-2.5)]);
	assert_eq!(format!("{half:?}"), "HalfVector<2>[1.0, -2.5]");

	let mut bits = [false; 8];
	bits[2] = true;
	assert_eq!(
		format!("{:?}", BitVector::from_bits(bits)),
		"BitVector<8>(00100000)"
	);
}

#[test]
fn bit_text_has_one_digit_per_bit() {
	assert_eq!(
		serde_json::from_str::<BitVector<8>>("\"0101\"")
			.unwrap_err()
			.to_string(),
		"a BitVector<8> has 8 bits, not 4"
	);
	assert_eq!(
		serde_json::from_str::<BitVector<8>>("\"0101010x\"")
			.unwrap_err()
			.to_string(),
		"a BitVector's bits are written as `0` and `1`"
	);
}

#[test]
fn a_sparse_value_must_be_finite() {
	assert_eq!(
		SparseVector::<4>::new([(1, f32::INFINITY)]).unwrap_err(),
		"the value at 1, inf, isn't finite"
	);
}

#[cfg(feature = "server")]
mod stored {
	use super::*;
	use crate::Column;
	use crate::Value;

	fn round_trip<C: Column + PartialEq + fmt::Debug>(column: &C) -> Vec<u8> {
		let Value::Bytes(bytes) = column.to_value().unwrap() else {
			panic!("a vector is stored as bytes");
		};
		assert_eq!(&C::from_value(Value::Bytes(bytes.clone())).unwrap(), column);
		bytes
	}

	#[test]
	fn every_kind_round_trips_through_its_stored_bytes() {
		let vector = Vector::new([1.0, -2.0, 0.5]);
		assert_eq!(round_trip(&vector).len(), 12);
		assert_eq!(round_trip(&HalfVector::from(&vector)).len(), 12);
		assert_eq!(
			round_trip(&BitVector::<16>::from_bytes(&[0xF0, 0x01]).unwrap()),
			[0xF0, 0x01]
		);

		// pgvector's binary form: dimensions, entries, a reserved zero, then
		// each index and each value, big-endian.
		let sparse = SparseVector::<10>::new([(7, 2.5), (1, -1.0)]).unwrap();
		let mut expected = Vec::new();
		for word in [10_u32, 2, 0, 1, 7] {
			expected.extend_from_slice(&word.to_be_bytes());
		}
		for value in [-1.0_f32, 2.5] {
			expected.extend_from_slice(&value.to_be_bytes());
		}
		assert_eq!(round_trip(&sparse), expected);
		assert_eq!(round_trip(&SparseVector::<10>::new([]).unwrap()).len(), 12);
	}

	fn refused<C: Column + fmt::Debug>(value: Value) -> String {
		C::from_value(value).unwrap_err().to_string()
	}

	#[test]
	fn a_stored_value_of_the_wrong_shape_is_refused() {
		assert!(
			refused::<Vector<3>>(Value::Bytes(vec![0; 8]))
				.contains("a Vector<3> is 12 bytes, and the stored value is 8"),
		);
		assert!(
			refused::<HalfVector<2>>(Value::Bytes(vec![0; 4]))
				.contains("is 8 bytes, and the stored value is 4"),
		);
		assert!(
			refused::<BitVector<16>>(Value::Bytes(vec![0; 3]))
				.contains("a BitVector<16> is 2 bytes, and the stored value is 3"),
		);
		assert!(refused::<Vector<3>>(Value::Int(3)).contains("expected a vector"));

		let sparse = SparseVector::<10>::new([(1, 1.0)])
			.unwrap()
			.to_value()
			.unwrap();
		let Value::Bytes(bytes) = sparse else {
			unreachable!()
		};
		assert!(
			refused::<SparseVector<12>>(Value::Bytes(bytes.clone())).contains(
				"the stored sparse vector has 10 dimensions, and the column's type expects 12"
			)
		);
		assert!(
			refused::<SparseVector<10>>(Value::Bytes(bytes[..6].to_vec()))
				.contains("the stored sparse vector is cut short")
		);
		assert!(
			refused::<SparseVector<10>>(Value::Bytes(bytes[..16].to_vec()))
				.contains("the stored sparse vector is the wrong length")
		);
	}

	#[test]
	fn a_vector_that_is_not_finite_is_not_stored() {
		let error = Vector::new([f32::NAN]).to_value().unwrap_err().to_string();
		assert!(error.contains("finite"), "{error}");
	}
}

#[cfg(feature = "postgres")]
mod pgvector {
	use super::super::column::from_postgres;
	use super::super::column::to_postgres;
	use super::*;
	use crate::Column;
	use crate::Value;

	fn stored(column: &impl Column) -> Vec<u8> {
		match column.to_value().unwrap() {
			Value::Bytes(bytes) => bytes,
			other => panic!("{other:?}"),
		}
	}

	#[test]
	fn every_kind_converts_to_pgvector_and_back() {
		let vector = Vector::new([1.0, -2.0, 0.5]);
		let bits = BitVector::<16>::from_bytes(&[0b0000_0001, 0b1000_0000]).unwrap();
		let sparse = SparseVector::<10>::new([(3, 4.0)]).unwrap();

		for (ty, bytes) in [
			("vector", stored(&vector)),
			("halfvec", stored(&HalfVector::from(&vector))),
			("bit", stored(&bits)),
			("varbit", stored(&bits)),
			("sparsevec", stored(&sparse)),
		] {
			let wire = to_postgres(ty, &bytes).unwrap().unwrap();
			assert_eq!(from_postgres(ty, &wire).unwrap().unwrap(), bytes, "{ty}");
		}

		// A float vector leads with its dimensions; a bit string with its
		// length in bits, and the first bit in each byte's highest.
		assert_eq!(
			to_postgres("vector", &stored(&vector)).unwrap().unwrap()[..4],
			[0, 3, 0, 0]
		);
		assert_eq!(
			to_postgres("bit", &stored(&bits)).unwrap().unwrap(),
			[0, 0, 0, 16, 0b1000_0000, 0b0000_0001]
		);
		assert_eq!(to_postgres("text", &[]), None);
		assert_eq!(from_postgres("text", &[]), None);
	}

	#[test]
	fn a_malformed_pgvector_value_is_refused() {
		assert_eq!(
			from_postgres("vector", &[0, 1]).unwrap(),
			Err("pgvector sent a vector without its dimensions".to_owned())
		);
		assert_eq!(
			from_postgres("halfvec", &[0, 2, 0, 0, 0, 0]).unwrap(),
			Err("pgvector sent a vector of the wrong length".to_owned())
		);
		assert_eq!(
			from_postgres("bit", &[0, 0]).unwrap(),
			Err("Postgres sent a bit string without its length".to_owned())
		);
	}
}

#[cfg(feature = "sqlite")]
mod sqlite_functions {
	use rusqlite::Connection;

	fn connection() -> Connection {
		let connection = Connection::open_in_memory().unwrap();
		super::super::sqlite::register(&connection).unwrap();
		connection
	}

	fn floats(values: &[f32]) -> Vec<u8> {
		values
			.iter()
			.flat_map(|value| value.to_le_bytes())
			.collect()
	}

	#[test]
	fn the_inner_product_and_jaccard_distances_measure_as_pgvector_does() {
		let connection = connection();
		let ip: f64 = connection
			.query_row(
				"SELECT lk_vec_distance_ip(?1, ?2)",
				(floats(&[1.0, 2.0]), floats(&[3.0, -1.0])),
				|row| row.get(0),
			)
			.unwrap();
		assert_eq!(ip, -1.0);

		let jaccard: f64 = connection
			.query_row(
				"SELECT lk_vec_distance_jaccard(?1, ?2)",
				(vec![0b0000_0011_u8], vec![0b0000_0110_u8]),
				|row| row.get(0),
			)
			.unwrap();
		assert!((jaccard - 2.0 / 3.0).abs() < 1e-9, "{jaccard}");

		let empty: f64 = connection
			.query_row("SELECT lk_vec_distance_jaccard(x'00', x'00')", [], |row| {
				row.get(0)
			})
			.unwrap();
		assert_eq!(empty, 1.0, "two empty sets are as far apart as can be");
	}

	#[test]
	fn a_null_vector_has_no_distance() {
		let connection = connection();
		for sql in [
			"SELECT lk_vec_distance_ip(NULL, x'0000803f')",
			"SELECT lk_vec_distance_jaccard(x'01', NULL)",
		] {
			let distance: Option<f64> = connection.query_row(sql, [], |row| row.get(0)).unwrap();
			assert_eq!(distance, None, "{sql}");
		}
	}

	#[test]
	fn vectors_that_cannot_be_compared_are_errors() {
		let connection = connection();
		for (sql, reason) in [
			(
				"SELECT lk_vec_distance_ip('one', x'0000803f')",
				"a vector is a BLOB",
			),
			(
				"SELECT lk_vec_distance_ip(x'000080', x'0000803f')",
				"a float vector is a whole number of 4-byte floats",
			),
			(
				"SELECT lk_vec_distance_ip(x'0000803f', x'0000803f0000803f')",
				"the vectors have different dimensions",
			),
			(
				"SELECT lk_vec_distance_jaccard(x'01', x'0101')",
				"the vectors have different dimensions",
			),
		] {
			let error = connection
				.query_row(sql, [], |row| row.get::<_, f64>(0))
				.unwrap_err()
				.to_string();
			assert!(error.contains(reason), "{sql}: {error}");
		}
	}

	#[test]
	fn the_fingerprint_is_the_texts_sha256() {
		let connection = connection();
		let fingerprint: Option<String> = connection
			.query_row("SELECT lk_sha256('abc')", [], |row| row.get(0))
			.unwrap();
		assert_eq!(
			fingerprint.as_deref(),
			Some("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
		);
		let none: Option<String> = connection
			.query_row("SELECT lk_sha256(42)", [], |row| row.get(0))
			.unwrap();
		assert_eq!(none, None);
	}
}
