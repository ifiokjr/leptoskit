use std::sync::Arc;

use leptoskit_rive::Asset;
use leptoskit_rive::Error;
use leptoskit_rive::Input;
use leptoskit_rive::MAX_ASSET_BYTES;
use leptoskit_rive::Name;

const HEADER: &[u8] = b"RIVE\x07\x00\x00\x00";

#[test]
fn shared_assets_keep_the_same_allocation() {
	let bytes: Arc<[u8]> = HEADER.into();
	let asset = Asset::from_shared(Arc::clone(&bytes)).unwrap();
	let clone = asset.clone();
	assert!(std::ptr::eq(bytes.as_ptr(), asset.bytes().as_ptr()));
	assert!(std::ptr::eq(asset.bytes().as_ptr(), clone.bytes().as_ptr()));
	assert_eq!(asset, Asset::new(HEADER).unwrap());
}

#[test]
fn rejects_truncated_headers_and_overflow_without_panicking() {
	for length in 0..HEADER.len() {
		assert_eq!(Asset::new(&HEADER[..length]), Err(Error::InvalidHeader));
	}

	for bytes in [
		b"nope\x07\x00\x00\x00".as_slice(),
		b"RIVE\x87\x00\x00\x00\x00",
		b"RIVE\xff\xff\xff\xff\x10\x00\x00\x00",
		b"RIVE\xff\xff\xff\xff\xff\x00\x00\x00",
		b"RIVE\x07\x80",
		b"RIVE\x07\x00\x80",
	] {
		assert_eq!(Asset::new(bytes), Err(Error::InvalidHeader));
	}

	assert_eq!(
		Asset::new(b"RIVE\x08\x00\x00\x00"),
		Err(Error::UnsupportedVersion(8))
	);
	assert_eq!(
		Asset::new(b"RIVE\xff\xff\xff\xff\x0f\x00\x00\x00"),
		Err(Error::UnsupportedVersion(u32::MAX))
	);
}

#[test]
fn rejects_oversized_files_before_import() {
	let mut bytes = vec![0; MAX_ASSET_BYTES];
	bytes[..HEADER.len()].copy_from_slice(HEADER);
	let asset = Asset::new(&bytes).unwrap();
	assert!(format!("{asset:?}").len() < 80);
	bytes.push(0);
	assert_eq!(Asset::new(&bytes), Err(Error::AssetTooLarge));
	assert_eq!(Asset::from_shared(bytes.into()), Err(Error::AssetTooLarge));
}

#[test]
fn runtime_names_and_numbers_have_explicit_limits() {
	for name in ["", "\0", "line\nbreak", "\u{7f}"] {
		assert_eq!(Name::new(name), Err(Error::InvalidName));
	}

	assert!(Name::new("é".repeat(128)).is_ok());
	assert_eq!(Name::new("é".repeat(129)), Err(Error::InvalidName));
	assert_eq!(Name::new("rating 🌟").unwrap().as_str(), "rating 🌟");

	for number in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, f64::MAX] {
		assert_eq!(Input::Number(number).validate(), Err(Error::InvalidNumber));
	}

	for input in [
		Input::Boolean(true),
		Input::Trigger,
		Input::Number(0.0),
		Input::Number(f64::from(f32::MAX)),
	] {
		assert!(input.validate().is_ok());
	}
}

#[test]
fn native_pixel_views_validate_layout_without_copying() {
	use leptoskit_rive::Frame;
	let bytes = [0; 16];
	let frame = Frame::new(2, 2, &bytes).unwrap();
	assert_eq!((frame.width(), frame.height(), frame.stride()), (2, 2, 8));
	assert_eq!(frame.bytes().as_ptr(), bytes.as_ptr());
	for (width, height, data) in [
		(0, 1, &bytes[..]),
		(1, 1, &bytes[..]),
		(4097, 1, &bytes[..]),
		(4096, 4096, &bytes[..]),
	] {
		assert_eq!(
			Frame::new(width, height, data).unwrap_err(),
			Error::InvalidFrame
		);
	}
}

#[test]
fn replacing_bindings_is_atomic_and_keeps_the_selected_artboard() {
	use std::collections::BTreeMap;

	use leptoskit_rive::Playback;
	use leptoskit_rive::Update;
	let mut playback =
		Playback::new(Asset::new(HEADER).unwrap()).artboard(Name::new("Artboard").unwrap());
	let name = Name::new("amount").unwrap();
	playback
		.set_input(name.clone(), Input::Number(0.5), 1)
		.unwrap();
	let original = playback.clone();
	let invalid = BTreeMap::from([(
		name.clone(),
		Update {
			value: Input::Number(f64::NAN),
			sequence: 2,
		},
	)]);
	assert_eq!(playback.replace_inputs(invalid), Err(Error::InvalidNumber));
	assert_eq!(playback, original);
	let oversized = (0..65)
		.map(|index| {
			(
				Name::new(format!("input{index}")).unwrap(),
				Update {
					value: Input::Trigger,
					sequence: 1,
				},
			)
		})
		.collect();
	assert!(playback.replace_inputs(oversized).is_err());
	assert_eq!(playback, original);
	playback.replace_inputs(BTreeMap::new()).unwrap();
	assert!(playback.inputs().is_empty());
	assert_eq!(playback.artboard_name().unwrap().as_str(), "Artboard");
	assert!(playback.same_scene(&original));
}

#[cfg(feature = "serde")]
#[test]
fn serialized_playback_preserves_validated_assets_and_rejects_invalid_bindings() {
	use leptoskit_rive::Playback;
	let mut playback = Playback::new(Asset::new(HEADER).unwrap());
	playback
		.set_input(Name::new("amount").unwrap(), Input::Number(0.5), 0)
		.unwrap();
	let value = serde_json::to_value(&playback).unwrap();
	assert_eq!(
		serde_json::from_value::<Playback>(value.clone()).unwrap(),
		playback
	);
	let mut invalid = value.clone();
	invalid["inputs"]["amount"]["value"] = serde_json::json!({"Number": f64::MAX});
	assert!(serde_json::from_value::<Playback>(invalid).is_err());
	let mut invalid = value.clone();
	invalid["asset"] = "not base64".into();
	assert!(serde_json::from_value::<Playback>(invalid).is_err());
	let mut invalid = value.clone();
	invalid["artboard"] = "".into();
	assert!(serde_json::from_value::<Playback>(invalid).is_err());
	let mut invalid = value;
	invalid["inputs"] = serde_json::Value::Object(
		(0..65)
			.map(|index| {
				(
					format!("input{index}"),
					serde_json::json!({"value": "Trigger", "sequence": 0}),
				)
			})
			.collect(),
	);
	assert!(serde_json::from_value::<Playback>(invalid).is_err());
}
