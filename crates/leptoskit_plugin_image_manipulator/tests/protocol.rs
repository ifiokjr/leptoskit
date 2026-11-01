use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_plugin_image_manipulator::*;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;
#[cfg(target_arch = "wasm32")]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

fn header() -> Vec<u8> {
	let mut bytes = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR".to_vec();
	bytes.extend_from_slice(&1_u32.to_be_bytes());
	bytes.extend_from_slice(&1_u32.to_be_bytes());
	bytes.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
	bytes
}

#[test]
fn decoded_image_retains_reply_storage_without_copying() {
	let expected = header();
	let reply = EncodedImage {
		info: inspect(&expected).unwrap(),
		bytes: Bytes::copy_from_slice(&expected),
	}
	.encode_reply()
	.unwrap();
	let image_start = reply[20..].as_ptr();
	let image_length = reply.len() - 20;
	let decoded = EncodedImage::decode_reply(reply.clone()).unwrap();
	assert_eq!(decoded.bytes.as_ptr(), image_start);
	assert_eq!(decoded.bytes.len(), image_length);
	drop(reply);

	assert_eq!(decoded.bytes.as_ref(), expected);
	assert_eq!(inspect(&decoded.bytes), Ok(decoded.info));
}

#[test]
fn guest_header_validation_needs_no_pixel_decoder() {
	let input = header();
	let info = ImageInfo {
		format: Format::Png,
		width: 1,
		height: 1,
	};
	assert_eq!(inspect(&input), Ok(info));
	let image = EncodedImage {
		info,
		bytes: Bytes::copy_from_slice(&input),
	};
	let reply = image.encode_reply().unwrap();
	assert_eq!(EncodedImage::decode_reply(reply.clone()), Ok(image.clone()));

	for length in 0..reply.len() {
		assert!(EncodedImage::decode_reply(reply.slice(..length)).is_err());
	}

	let mut invalid = reply.to_vec();
	invalid.push(0);
	assert_eq!(
		EncodedImage::decode_reply(invalid.into()),
		Err(Error::InvalidArgument)
	);
	let mut mismatched = image;
	mismatched.info.width = 2;
	assert_eq!(
		EncodedImage::decode_reply(mismatched.encode_reply().unwrap()),
		Err(Error::InvalidArgument)
	);
	let mut oversized = input;
	oversized[16..20].copy_from_slice(&u32::MAX.to_be_bytes());

	assert_eq!(inspect(&oversized), Err(Error::TooLarge));
	assert_eq!(
		EncodedImage::decode_reply(vec![0; MAX_OUTPUT_BYTES + 21].into()),
		Err(Error::TooLarge)
	);
}

#[test]
fn guest_request_bounds_and_invalid_options_fail_locally() {
	assert_eq!(
		encode_request(&vec![0; MAX_INPUT_BYTES + 1], &[], Output::Png),
		Err(Error::TooLarge)
	);
	assert_eq!(
		encode_request(
			&header(),
			&vec![Operation::Flip(Axis::Vertical); MAX_OPERATIONS + 1],
			Output::Png,
		),
		Err(Error::TooLarge)
	);
	assert_eq!(
		encode_request(
			&header(),
			&[Operation::Resize {
				width: None,
				height: None,
				filter: Filter::Nearest,
			}],
			Output::Png,
		),
		Err(Error::InvalidArgument)
	);
	assert_eq!(
		encode_request(
			&header(),
			&[],
			Output::Jpeg {
				quality: 0,
				background: [0; 3],
			},
		),
		Err(Error::InvalidArgument)
	);
}

#[test]
fn jpeg_headers_are_bounded_before_any_decoder_exists() {
	// Fill bytes, an empty APP0 segment and a standalone marker precede the
	// frame header: precision, height 2, width 3, then one component.
	let frame = |marker: u8, precision: u8| {
		vec![
			0xff, 0xd8, 0xff, 0xe0, 0, 2, 0xff, 0x01, 0xff, 0xff, marker, 0, 11, precision, 0, 2,
			0, 3, 1, 1, 0x11, 0,
		]
	};
	let info = ImageInfo {
		format: Format::Jpeg,
		width: 3,
		height: 2,
	};
	assert_eq!(inspect(&frame(0xc0, 8)), Ok(info));
	assert_eq!(inspect(&frame(0xc2, 8)), Ok(info));
	// Lossless, arithmetic-coded and 12-bit frames have no supported decoder.
	for (marker, precision) in [(0xc3, 8), (0xc9, 8), (0xc0, 12)] {
		assert_eq!(inspect(&frame(marker, precision)), Err(Error::Unavailable));
	}
	for invalid in [
		&[0xff, 0xd8, 0x00][..],
		&[0xff, 0xd8, 0xff],
		&[0xff, 0xd8, 0xff, 0xda, 0, 2],
		&[0xff, 0xd8, 0xff, 0xe0, 0, 1],
		&[0xff, 0xd8, 0xff, 0xe0, 0, 2],
	] {
		assert_eq!(
			inspect(invalid),
			Err(Error::InvalidArgument),
			"{invalid:x?}"
		);
	}
}
