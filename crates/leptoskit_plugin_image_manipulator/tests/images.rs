use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::Plugin;
use leptoskit_plugin_image_manipulator::*;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;
#[cfg(target_arch = "wasm32")]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

fn sample() -> Bitmap {
	Bitmap::from_rgba(3, 2, (1..=6).flat_map(|value| [value, 0, 0, 255]).collect()).unwrap()
}
fn red(bitmap: &Bitmap) -> Vec<u8> {
	bitmap
		.rgba()
		.as_chunks::<4>()
		.0
		.iter()
		.map(|pixel| pixel[0])
		.collect()
}
fn png_bytes() -> Bytes {
	encode(&sample(), Output::Png).unwrap().bytes
}

#[test]
fn png_and_jpeg_roundtrip_real_codecs() {
	let sample = sample();
	let png = encode(&sample, Output::Png).unwrap();
	assert_eq!(decode(&png.bytes).unwrap(), sample);
	let jpeg = encode(
		&sample,
		Output::Jpeg {
			quality: 90,
			background: [255; 3],
		},
	)
	.unwrap();
	assert_eq!(
		inspect(&jpeg.bytes).unwrap(),
		ImageInfo {
			format: Format::Jpeg,
			width: 3,
			height: 2
		}
	);
	let decoded = decode(&jpeg.bytes).unwrap();
	assert_eq!((decoded.width(), decoded.height()), (3, 2));
	assert!(
		decoded
			.rgba()
			.as_chunks::<4>()
			.0
			.iter()
			.all(|pixel| pixel[3] == 255)
	);

	let converted = manipulate(&jpeg.bytes, &[], Output::Png).unwrap();
	assert_eq!(decode(&converted.bytes).unwrap(), decoded);
}

#[test]
fn rotations_and_flips_preserve_pixel_placement() {
	for (operation, expected, dimensions) in [
		(
			Operation::Rotate(Rotation::Clockwise90),
			vec![4, 1, 5, 2, 6, 3],
			(2, 3),
		),
		(
			Operation::Rotate(Rotation::HalfTurn),
			vec![6, 5, 4, 3, 2, 1],
			(3, 2),
		),
		(
			Operation::Rotate(Rotation::Counterclockwise90),
			vec![3, 6, 2, 5, 1, 4],
			(2, 3),
		),
		(
			Operation::Flip(Axis::Horizontal),
			vec![3, 2, 1, 6, 5, 4],
			(3, 2),
		),
		(
			Operation::Flip(Axis::Vertical),
			vec![4, 5, 6, 1, 2, 3],
			(3, 2),
		),
	] {
		let bitmap = sample().transform(&[operation]).unwrap();
		assert_eq!(red(&bitmap), expected);
		assert_eq!((bitmap.width(), bitmap.height()), dimensions);
	}
}

#[test]
fn ordered_crop_resize_and_binary_bridge_agree() {
	let input = png_bytes();
	let operations = [
		Operation::Rotate(Rotation::Clockwise90),
		Operation::Crop {
			x: 0,
			y: 1,
			width: 2,
			height: 2,
		},
		Operation::Flip(Axis::Vertical),
		Operation::Resize {
			width: Some(4),
			height: None,
			filter: Filter::Nearest,
		},
	];
	let expected = manipulate(&input, &operations, Output::Png).unwrap();
	let result = ImageManipulator
		.invoke(
			MANIPULATE,
			encode_request(&input, &operations, Output::Png).unwrap(),
		)
		.unwrap();
	let decoded = EncodedImage::decode_reply(result).unwrap();
	assert_eq!(decoded, expected);
	let pixels = decode(&decoded.bytes).unwrap();
	assert_eq!((pixels.width(), pixels.height()), (4, 4));

	assert_eq!(
		red(&pixels),
		[6, 6, 3, 3, 6, 6, 3, 3, 5, 5, 2, 2, 5, 5, 2, 2]
	);
	let header = ImageManipulator.invoke(INSPECT, input).unwrap();
	assert_eq!(
		ImageInfo::decode(&header).unwrap(),
		ImageInfo {
			format: Format::Png,
			width: 3,
			height: 2
		}
	);
}

#[test]
fn bilinear_resize_uses_premultiplied_alpha() {
	let pixels = Bitmap::from_rgba(2, 1, vec![255, 0, 0, 0, 0, 0, 255, 255]).unwrap();
	let resized = pixels
		.transform(&[Operation::Resize {
			width: Some(1),
			height: Some(1),
			filter: Filter::Bilinear,
		}])
		.unwrap();
	assert_eq!(resized.rgba(), [0, 0, 255, 128]);
	let clear = Bitmap::from_rgba(1, 1, vec![255, 0, 0, 0])
		.unwrap()
		.transform(&[Operation::Resize {
			width: Some(2),
			height: Some(2),
			filter: Filter::Bilinear,
		}])
		.unwrap();
	assert!(clear.rgba().iter().all(|byte| *byte == 0));
}

#[test]
fn jpeg_alpha_uses_explicit_background_and_quality_is_validated() {
	let bitmap = Bitmap::from_rgba(1, 1, vec![255, 0, 0, 0]).unwrap();
	let encoded = encode(
		&bitmap,
		Output::Jpeg {
			quality: 100,
			background: [255; 3],
		},
	)
	.unwrap();
	let result = decode(&encoded.bytes).unwrap();
	assert!(result.rgba()[..3].iter().all(|byte| *byte >= 250));

	for quality in [0, 101, 255] {
		assert_eq!(
			encode(
				&bitmap,
				Output::Jpeg {
					quality,
					background: [0; 3]
				}
			),
			Err(Error::InvalidArgument)
		);
	}
}

#[test]
fn invalid_plan_fails_before_decoding_compressed_pixels() {
	let mut input = png_bytes().to_vec();
	// Keep a valid header but corrupt the payload; preflight must reject the
	// plan first.
	input[40] ^= 255;
	for operation in [
		Operation::Crop {
			x: u32::MAX,
			y: 0,
			width: 1,
			height: 1,
		},
		Operation::Crop {
			x: 2,
			y: 0,
			width: 2,
			height: 1,
		},
		Operation::Resize {
			width: None,
			height: None,
			filter: Filter::Nearest,
		},
		Operation::Resize {
			width: Some(0),
			height: Some(1),
			filter: Filter::Bilinear,
		},
	] {
		assert_eq!(
			manipulate(&input, &[operation], Output::Png),
			Err(Error::InvalidArgument)
		);
	}
	assert_eq!(
		manipulate(
			&input,
			&[Operation::Resize {
				width: Some(MAX_DIMENSION + 1),
				height: Some(1),
				filter: Filter::Nearest
			}],
			Output::Png
		),
		Err(Error::TooLarge)
	);
	assert_eq!(
		manipulate(
			&input,
			&vec![Operation::Flip(Axis::Horizontal); MAX_OPERATIONS + 1],
			Output::Png
		),
		Err(Error::TooLarge)
	);
	// A valid-size header with invalid CRC/pixels proves cumulative work is
	// checked before constructing the decoder or allocating the image.
	input[16..20].copy_from_slice(&2048_u32.to_be_bytes());
	input[20..24].copy_from_slice(&2048_u32.to_be_bytes());
	assert_eq!(
		manipulate(&input, &[Operation::Flip(Axis::Horizontal); 8], Output::Png),
		Err(Error::TooLarge)
	);
}

#[test]
fn malicious_dimensions_are_rejected_before_decode_allocation() {
	let mut png = png_bytes().to_vec();

	for (width, height) in [(u32::MAX, 1), (1, u32::MAX), (MAX_DIMENSION, MAX_DIMENSION)] {
		png[16..20].copy_from_slice(&width.to_be_bytes());
		png[20..24].copy_from_slice(&height.to_be_bytes());
		assert_eq!(decode(&png), Err(Error::TooLarge));
	}

	let jpeg = [
		0xff, 0xd8, 0xff, 0xc0, 0, 11, 8, 0xff, 0xff, 0xff, 0xff, 1, 1, 0x11, 0, 0xff, 0xd9,
	];
	assert_eq!(decode(&jpeg), Err(Error::TooLarge));
	assert_eq!(
		Bitmap::from_rgba(u32::MAX, u32::MAX, vec![]),
		Err(Error::TooLarge)
	);

	assert_eq!(
		Bitmap::from_rgba(1, 1, vec![0; 3]),
		Err(Error::InvalidArgument)
	);
	assert_eq!(decode(&vec![0; MAX_INPUT_BYTES + 1]), Err(Error::TooLarge));
}

#[test]
fn truncated_corrupt_and_unsupported_images_fail() {
	let png = png_bytes();

	for length in 0..png.len() {
		assert!(decode(&png[..length]).is_err(), "prefix {length}");
	}

	let jpeg = encode(
		&sample(),
		Output::Jpeg {
			quality: 80,
			background: [0; 3],
		},
	)
	.unwrap();
	assert!(decode(&jpeg.bytes[..jpeg.bytes.len() / 2]).is_err());
	assert!(decode(&jpeg.bytes[..jpeg.bytes.len() - 2]).is_err());
	assert_eq!(decode(b"GIF89a"), Err(Error::Unavailable));
	let mut corrupt = png.to_vec();

	corrupt[32] ^= 255;
	assert!(decode(&corrupt).is_err());
}

#[test]
fn jpeg_data_after_the_end_of_image_marker_is_ignored() {
	let jpeg = encode(
		&sample(),
		Output::Jpeg {
			quality: 90,
			background: [0; 3],
		},
	)
	.unwrap();
	let expected = decode(&jpeg.bytes).unwrap();
	// Motion photos append a video after the JPEG; vendors append trailers.
	let mut motion = jpeg.bytes.to_vec();
	motion.extend_from_slice(b"\0\0\0\x18ftypmp42\xff\xd8\xff\xd9trailer");
	assert_eq!(decode(&motion).unwrap(), expected);
	assert_eq!(
		manipulate(&motion, &[], Output::Png).unwrap().info,
		ImageInfo {
			format: Format::Png,
			width: expected.width(),
			height: expected.height(),
		}
	);
	// A segment before the scan, like an EXIF thumbnail, may contain its own
	// end-of-image bytes.
	let mut thumbnail = vec![0xff, 0xd8, 0xff, 0xe1, 0, 6, 0xff, 0xd9, 0xff, 0xd9];
	thumbnail.extend_from_slice(&jpeg.bytes[2..]);

	assert_eq!(decode(&thumbnail).unwrap(), expected);
	let mut unterminated = jpeg.bytes[..jpeg.bytes.len() - 2].to_vec();
	unterminated.extend_from_slice(b"trailer");
	assert!(decode(&unterminated).is_err());
}

#[test]
fn compressed_metadata_is_not_propagated_and_animation_is_rejected() {
	let mut bytes = Vec::new();
	{
		let mut encoder = png::Encoder::new(&mut bytes, 1, 1);
		encoder.set_color(png::ColorType::Rgba);
		encoder.set_depth(png::BitDepth::Eight);
		encoder
			.add_ztxt_chunk("comment".into(), "x".repeat(MAX_CODEC_BYTES as usize + 1))
			.unwrap();
		encoder
			.write_header()
			.unwrap()
			.write_image_data(&[1, 2, 3, 255])
			.unwrap();
	}
	assert!(bytes.len() < MAX_INPUT_BYTES);

	// Text remains compressed inside the PNG decoder and is never requested by
	// the pixel API. A large text expansion must not be propagated to output.
	let decoded = decode(&bytes).unwrap();
	assert_eq!(decoded.rgba(), [1, 2, 3, 255]);
	let stripped = encode(&decoded, Output::Png).unwrap();
	assert!(!stripped.bytes.windows(4).any(|chunk| chunk == b"zTXt"));
	let mut animated = Vec::new();
	{
		let mut encoder = png::Encoder::new(&mut animated, 1, 1);
		encoder.set_color(png::ColorType::Rgba);
		encoder.set_depth(png::BitDepth::Eight);
		encoder.set_animated(2, 0).unwrap();
		let mut writer = encoder.write_header().unwrap();
		writer.write_image_data(&[1, 2, 3, 255]).unwrap();
		writer.write_image_data(&[4, 5, 6, 255]).unwrap();
	}
	assert_eq!(decode(&animated), Err(Error::Unavailable));
}

#[test]
fn wire_decoders_reject_truncation_trailing_bytes_and_mismatched_headers() {
	let request = encode_request(&png_bytes(), &[], Output::Png).unwrap();

	for length in 0..request.len() {
		assert!(
			ImageManipulator
				.invoke(MANIPULATE, Bytes::copy_from_slice(&request[..length]))
				.is_err()
		);
	}

	let mut trailing = request.to_vec();
	trailing.push(0);
	assert_eq!(
		ImageManipulator.invoke(MANIPULATE, trailing.into()),
		Err(Error::InvalidArgument)
	);
	let mut result = encode(&sample(), Output::Png).unwrap();
	result.info.width = 2;

	assert_eq!(
		EncodedImage::decode_reply(result.encode_reply().unwrap()),
		Err(Error::InvalidArgument)
	);
	let result = encode(&sample(), Output::Png)
		.unwrap()
		.encode_reply()
		.unwrap();

	for length in 0..result.len() {
		assert!(EncodedImage::decode_reply(result.slice(..length)).is_err());
	}

	let mut invalid = result.to_vec();
	invalid[4..8].copy_from_slice(&99_u32.to_le_bytes());
	assert_eq!(
		EncodedImage::decode_reply(invalid.into()),
		Err(Error::InvalidArgument)
	);
}

#[test]
fn every_operation_and_output_crosses_the_binary_bridge() {
	let mut plugin = ImageManipulator;
	assert_eq!(plugin.name(), "image_manipulator");
	assert_eq!(plugin.methods(), &[INSPECT, MANIPULATE]);
	assert_eq!(
		plugin.invoke(leptoskit_bridge::MethodId(2), Bytes::new()),
		Err(Error::Unavailable)
	);
	assert_eq!(
		ImageInfo::decode(&plugin.invoke(INSPECT, png_bytes()).unwrap()),
		inspect(&png_bytes())
	);
	let operations = [
		Operation::Resize {
			width: Some(4),
			height: None,
			filter: Filter::Bilinear,
		},
		Operation::Resize {
			width: None,
			height: Some(2),
			filter: Filter::Nearest,
		},
		Operation::Rotate(Rotation::Clockwise90),
		Operation::Rotate(Rotation::HalfTurn),
		Operation::Rotate(Rotation::Counterclockwise90),
		Operation::Flip(Axis::Horizontal),
		Operation::Flip(Axis::Vertical),
		Operation::Crop {
			x: 0,
			y: 0,
			width: 1,
			height: 1,
		},
	];
	let output = Output::Jpeg {
		quality: 75,
		background: [10, 20, 30],
	};
	let request = encode_request(&png_bytes(), &operations, output).unwrap();

	let reply = EncodedImage::decode_reply(plugin.invoke(MANIPULATE, request).unwrap()).unwrap();
	assert_eq!(
		reply,
		manipulate(&png_bytes(), &operations, output).unwrap()
	);
	assert_eq!(reply.info.format, Format::Jpeg);
}

#[test]
fn native_requests_reject_unknown_tags_and_oversized_options() {
	let word = |request: &Bytes, offset: usize, value: u32| {
		let mut bytes = request.to_vec();
		bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
		Bytes::from(bytes)
	};
	let invoke = |payload: Bytes| ImageManipulator.invoke(MANIPULATE, payload);
	let rotate = encode_request(
		&png_bytes(),
		&[Operation::Rotate(Rotation::Clockwise90)],
		Output::Png,
	)
	.unwrap();
	// Output tag, operation count, operation tag, then the rotation.
	for (offset, value, expected) in [
		(0, 2, Error::InvalidArgument),
		(4, MAX_OPERATIONS as u32 + 1, Error::TooLarge),
		(8, 4, Error::InvalidArgument),
		(12, 3, Error::InvalidArgument),
	] {
		assert_eq!(invoke(word(&rotate, offset, value)), Err(expected));
	}
	let flip = encode_request(
		&png_bytes(),
		&[Operation::Flip(Axis::Vertical)],
		Output::Png,
	)
	.unwrap();
	assert_eq!(invoke(word(&flip, 12, 2)), Err(Error::InvalidArgument));
	let resize = encode_request(
		&png_bytes(),
		&[Operation::Resize {
			width: Some(1),
			height: Some(1),
			filter: Filter::Nearest,
		}],
		Output::Png,
	)
	.unwrap();
	assert_eq!(invoke(word(&resize, 20, 2)), Err(Error::InvalidArgument));

	let jpeg = encode_request(
		&png_bytes(),
		&[],
		Output::Jpeg {
			quality: 90,
			background: [0; 3],
		},
	)
	.unwrap();
	// Quality, then the first background component.
	for (offset, value) in [(4, 0), (4, 256), (8, 256)] {
		assert_eq!(
			invoke(word(&jpeg, offset, value)),
			Err(Error::InvalidArgument)
		);
	}

	assert_eq!(
		invoke(vec![0; MAX_INPUT_BYTES + MAX_OPERATIONS * 20 + 29].into()),
		Err(Error::TooLarge)
	);
}
