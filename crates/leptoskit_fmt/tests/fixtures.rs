//! Every file in `tests/fixtures` formats to its snapshot, and formatting
//! the snapshot again changes nothing.

mod support;

#[test]
fn fixtures_format_to_their_snapshots() {
	insta::glob!("fixtures/*.rs", |path| {
		let input = std::fs::read_to_string(path).expect("read fixture");
		let output = support::format(&input);
		let again = support::format(&output.text);
		assert_eq!(
			again.text,
			output.text,
			"formatting {} twice changed it",
			path.display()
		);
		insta::assert_snapshot!(support::describe(&output));
	});
}
