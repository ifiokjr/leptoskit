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
	assert_eq!(EncodedImage::decode_reply(&reply), Ok(image.clone()));
	for length in 0..reply.len() {
		assert!(EncodedImage::decode_reply(&reply[..length]).is_err());
	}
	let mut invalid = reply.to_vec();
	invalid.push(0);
	assert_eq!(
		EncodedImage::decode_reply(&invalid),
		Err(Error::InvalidArgument)
	);
	let mut mismatched = image;
	mismatched.info.width = 2;
	assert_eq!(
		EncodedImage::decode_reply(&mismatched.encode_reply().unwrap()),
		Err(Error::InvalidArgument)
	);
	let mut oversized = input;
	oversized[16..20].copy_from_slice(&u32::MAX.to_be_bytes());
	assert_eq!(inspect(&oversized), Err(Error::TooLarge));
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
