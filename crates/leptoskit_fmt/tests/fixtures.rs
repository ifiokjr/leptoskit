//! Every file in `tests/fixtures` formats to its snapshot, and formatting
//! the snapshot again changes nothing and runs rustfmt once.

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
		// A formatted file settles in one pass, and every expression in its
		// markup is predicted, so rustfmt runs once.
		assert_eq!(
			(again.stats.passes, again.stats.rustfmt_runs),
			(1, 1),
			"{}",
			path.display()
		);
		// Formatting the expressions in a second rustfmt process changes when
		// they are formatted, never how.
		let concurrent = support::RUSTFMT.clone().concurrent(true);
		let alongside =
			leptoskit_fmt::format_source(&input, &support::SETTINGS, &concurrent).expect("format");
		assert_eq!(alongside.text, output.text, "{}", path.display());
		insta::assert_snapshot!(support::describe(&output));
	});
}
