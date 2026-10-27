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
