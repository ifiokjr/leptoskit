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
