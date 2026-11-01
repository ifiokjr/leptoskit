use leptoskit_native::Element;
use leptoskit_testing::WidgetTester;
use leptoskit_testing::capture::LayoutImage;
use leptoskit_testing::capture::Screenshot;
use leptoskit_yoga::Dimension;

#[test]
fn real_capture_preserves_pixels_and_metadata_and_rejects_corruption() {
	let mut bytes = Vec::new();
	{
		let mut encoder = png::Encoder::new(&mut bytes, 2, 2);
		encoder.set_color(png::ColorType::Rgba);
		encoder
			.add_text_chunk("Description".to_owned(), "capture metadata".to_owned())
			.unwrap();
		encoder
			.write_header()
			.unwrap()
			.write_image_data(&[
				255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 128,
			])
			.unwrap();
	}
	let capture = Screenshot::from_png(bytes.clone()).unwrap();
	assert_eq!(capture.dimensions(), (2, 2));

	assert_eq!(capture.distinct_colors(8).unwrap(), 4);
	// Counting stops as soon as the limit is exceeded.
	assert_eq!(capture.distinct_colors(1).unwrap(), 2);
	assert_eq!(capture.png(), bytes);
	assert!(Screenshot::from_png(b"not a PNG".to_vec()).is_err());
	let mut missing_end = bytes.clone();
	missing_end.truncate(missing_end.len() - 12);
	assert!(Screenshot::from_png(missing_end).is_err());
	let mut corrupt_end = bytes.clone();
	*corrupt_end.last_mut().unwrap() ^= 1;
	assert!(Screenshot::from_png(corrupt_end).is_err());
	bytes.truncate(bytes.len() / 2);
	assert!(Screenshot::from_png(bytes).is_err());
}

#[test]
fn layout_diagnostic_is_marked_synthetic_and_excludes_hidden_controls() {
	let make = |hidden| {
		WidgetTester::new(
			move || {
				Element::view()
					.with_style(|style| {
						style.width = Dimension::Points(100.0);
						style.height = Dimension::Points(80.0);
					})
					.child(
						Element::button("Control")
							.with_props(|props| props.hidden = Some(hidden))
							.with_style(|style| {
								style.width = Dimension::Points(60.0);
								style.height = Dimension::Points(40.0);
							}),
					)
			},
			100.0,
			80.0,
		)
		.unwrap()
	};
	let visible = make(false).layout_image().unwrap();
	let hidden = make(true).layout_image().unwrap();
	assert_ne!(visible.png(), hidden.png());
	// Flat colour: the root covers the background, so only outlines and one
	// fill per visible widget remain.
	assert_eq!(visible.distinct_colors(LayoutImage::MAX_COLORS).unwrap(), 3);
	assert_eq!(hidden.distinct_colors(LayoutImage::MAX_COLORS).unwrap(), 2);
	let reader = png::Decoder::new(std::io::Cursor::new(visible.png()))
		.read_info()
		.unwrap();
	assert!(
		reader
			.info()
			.uncompressed_latin1_text
			.iter()
			.any(|text| text.text.contains("synthetic layout diagnostic"))
	);
}
