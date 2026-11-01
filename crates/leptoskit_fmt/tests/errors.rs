//! What happens when something cannot be formatted.

mod support;

use leptoskit_fmt::FormatError;
use leptoskit_fmt::Rustfmt;
use leptoskit_fmt::Settings;
use support::format;

#[test]
fn unparsable_markup_is_left_exactly_as_written() {
	let source = "fn f() {\n    let a = view! {  <p>\"ok\"</p>  };\n    let b = view! {\n  \
	              <div><p>\"unclosed\"</div>\n    };\n}\n";
	let output = format(source);
	assert!(
		output
			.text
			.contains("view! {\n  <div><p>\"unclosed\"</div>\n    }"),
		"{}",
		output.text
	);
	assert!(
		output.text.contains("let a = view! { <p>\"ok\"</p> };"),
		"{}",
		output.text
	);
	let [diagnostic] = output.diagnostics.as_slice() else {
		panic!("one diagnostic: {:?}", output.diagnostics);
	};
	assert_eq!(diagnostic.line, 4);
	assert!(
		diagnostic.message.starts_with("markup left as written:"),
		"{diagnostic}"
	);

	let again = format(&output.text);
	assert_eq!(
		(again.text, again.diagnostics),
		(output.text.clone(), output.diagnostics.clone()),
		"leaving markup alone is stable"
	);
}

#[test]
fn comments_the_layout_cannot_place_leave_the_markup_alone() {
	let source = "fn f() {\n\tview! { <a href /* here */ = \"x\" /> };\n}\n";
	let output = format(source);
	assert_eq!(output.text, source);
	assert!(
		output.diagnostics[0].message.contains("comment"),
		"{:?}",
		output.diagnostics
	);
}

#[test]
fn invalid_blocks_and_tokens_are_reported() {
	for markup in ["<a>{ let }</a>", "<a b={ ) }/>", "<a>\"unterminated</a>"] {
		let source = format!("fn f() {{\n\tview! {{ {markup} }};\n}}\n");

		match leptoskit_fmt::format_source(&source, &support::SETTINGS, &support::RUSTFMT) {
			Ok(output) => {
				assert_eq!(output.text, source, "{markup}");
				assert_eq!(
					output.diagnostics.len(),
					1,
					"{markup}: {:?}",
					output.diagnostics
				);
			}
			// Tokens rustfmt cannot lex fail the file instead.
			Err(error) => {
				assert!(
					matches!(error, FormatError::Rustfmt(_)),
					"{markup}: {error}"
				);
			}
		}
	}
}

#[test]
fn rust_rustfmt_rejects_is_kept_while_its_neighbours_format() {
	// The batch fails, so the expressions are split until the bad one is
	// alone; the good one is formatted all the same.
	let source = "fn f() {\n\tview! { <a>{ let }</a> <b>{x+y}</b> };\n}\n";
	let output = format(source);
	assert!(output.text.contains("{ let }"), "{}", output.text);
	assert!(output.text.contains("{x + y}"), "{}", output.text);
	assert_eq!(output.diagnostics.len(), 1, "{:?}", output.diagnostics);
	assert!(
		output.diagnostics[0]
			.message
			.contains("rustfmt could not parse"),
		"{:?}",
		output.diagnostics
	);
}

#[test]
fn rustup_resolves_a_toolchain_by_name() {
	// cargo runs the tests through rustup, which names its toolchain here.
	let Ok(toolchain) = std::env::var("RUSTUP_TOOLCHAIN") else {
		return;
	};
	let rustfmt = Rustfmt::locate(&toolchain).expect("the running toolchain has rustfmt");
	assert!(
		rustfmt.program().is_file(),
		"{}",
		rustfmt.program().display()
	);
	let output =
		leptoskit_fmt::format_source("fn  f() {}\n", &support::SETTINGS, &rustfmt).expect("format");
	assert_eq!(output.text, "fn f() {}\n");
}

#[test]
fn a_broken_nested_macro_is_kept_while_the_rest_formats() {
	let source =
		"fn f() {\n\tview! {   <ul>{items.map(|item| view! { <li>{item}</p> })}</ul>   };\n}\n";
	let output = format(source);
	assert!(
		output.text.contains("view! { <li>{item}</p> }"),
		"{}",
		output.text
	);
	assert!(
		output.text.contains("<ul>{items.map(|item|"),
		"{}",
		output.text
	);
	assert_eq!(output.diagnostics.len(), 1, "{:?}", output.diagnostics);
}

#[test]
fn rust_syntax_errors_fail_the_file() {
	let error = leptoskit_fmt::format_source(
		"fn f( {\n\tview! { <a/> }\n}\n",
		&support::SETTINGS,
		&support::RUSTFMT,
	)
	.expect_err("a syntax error");
	assert!(matches!(error, FormatError::Rustfmt(_)), "{error}");
	assert!(error.to_string().starts_with("rustfmt failed"), "{error}");
}

#[test]
fn a_missing_rustfmt_is_an_error() {
	let rustfmt = Rustfmt::at("/nonexistent/rustfmt");
	let error = leptoskit_fmt::format_source("fn f() {}\n", &Settings::default(), &rustfmt)
		.expect_err("no rustfmt");
	assert!(error.to_string().contains("could not run"), "{error}");
	let error = Rustfmt::locate("leptoskit-fmt-no-such-toolchain").expect_err("no toolchain");
	assert!(
		matches!(error, FormatError::RustfmtMissing { .. }),
		"{error}"
	);
	assert!(
		error.to_string().contains("rustup component add rustfmt"),
		"{error}"
	);
}

#[test]
fn invalid_configuration_is_reported() {
	let dir = std::env::temp_dir().join(format!("leptoskit-fmt-config-{}", std::process::id()));
	std::fs::create_dir_all(&dir).expect("temp dir");
	std::fs::write(
		dir.join("leptoskit.toml"),
		"[format]\nclosing_tags = \"sometimes\"\n",
	)
	.expect("write");
	let error = Settings::for_path(&dir.join("lib.rs")).expect_err("invalid closing_tags");
	assert!(matches!(error, FormatError::Config { .. }), "{error}");
	std::fs::write(dir.join("leptoskit.toml"), "[format]\n").expect("write");
	std::fs::write(dir.join("rustfmt.toml"), "max_width = \"wide\"\n").expect("write");
	let error = Settings::for_path(&dir.join("lib.rs")).expect_err("invalid max_width");

	assert!(error.to_string().contains("rustfmt.toml"), "{error}");
	std::fs::remove_dir_all(&dir).expect("clean up");
}
