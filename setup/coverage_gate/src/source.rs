//! Finding the test code inside source files.
//!
//! Unit tests live next to the code they test, in `#[cfg(test)]` modules.
//! Their lines run whenever the tests run, so counting them would let a change
//! raise its own coverage by adding tests rather than by covering its code.

use std::ops::RangeInclusive;

/// The 1-based line ranges of the `#[cfg(test)]` modules in `source`, from the
/// attribute to the module's closing brace.
///
/// The repository is formatted with rustfmt, so a module's closing brace is
/// the first later line that holds only `}` at the module's own indentation.
/// That rule needs no Rust parser; a raw string inside a test module with a
/// line that looks like that brace would end the range early, which only
/// counts some test lines as code.
pub fn test_modules(source: &str) -> Vec<RangeInclusive<u32>> {
	let lines: Vec<&str> = source.lines().collect();
	let mut ranges = Vec::new();
	let mut index = 0;
	while index < lines.len() {
		let line = lines[index];
		let indent = &line[..line.len() - line.trim_start().len()];
		if line.trim() != "#[cfg(test)]" {
			index += 1;
			continue;
		}
		// Further attributes may sit between `#[cfg(test)]` and the module.
		let mut item = index + 1;
		while item < lines.len() && lines[item].trim_start().starts_with("#[") {
			item += 1;
		}
		let Some(header) = lines.get(item) else {
			break;
		};
		let opens_module = header
			.strip_prefix(indent)
			.filter(|rest| !rest.starts_with(char::is_whitespace))
			.and_then(|rest| rest.strip_prefix("pub ").or(Some(rest)))
			.is_some_and(|rest| rest.starts_with("mod ") && rest.trim_end().ends_with('{'));
		if !opens_module {
			index = item;
			continue;
		}
		let closing = format!("{indent}}}");
		let end = lines[item + 1..]
			.iter()
			.position(|line| line.trim_end() == closing)
			.map_or(lines.len() - 1, |offset| item + 1 + offset);
		ranges.push(line_number(index)..=line_number(end));
		index = end + 1;
	}
	ranges
}

/// Whether a path names a whole file of tests: a `tests.rs` module, the
/// conventional file for an out-of-line `#[cfg(test)] mod tests;`.
pub fn is_test_file(path: &str) -> bool {
	path.rsplit('/').next() == Some("tests.rs")
}

fn line_number(index: usize) -> u32 {
	u32::try_from(index + 1).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn finds_top_level_and_nested_test_modules() {
		let source = [
			"fn code() {}",              // 1
			"",                          // 2
			"mod inner {",               // 3
			"\tfn more() {}",            // 4
			"\t#[cfg(test)]",            // 5
			"\t#[allow(unused)]",        // 6
			"\tmod checks {",            // 7
			"\t\t#[test]",               // 8
			"\t\tfn check() {}",         // 9
			"\t}",                       // 10
			"\tfn after() {}",           // 11
			"}",                         // 12
			"",                          // 13
			"#[cfg(test)]",              // 14
			"mod tests {",               // 15
			"\tfn helper() { if true {", // 16
			"\t}}",                      // 17
			"}",                         // 18
			"fn trailing() {}",          // 19
		]
		.join("\n");

		assert_eq!(test_modules(&source), [5..=10, 14..=18]);
	}

	#[test]
	fn ignores_test_items_that_are_not_inline_modules() {
		let source = [
			"#[cfg(test)]",
			"use std::fmt;",
			"#[cfg(test)]",
			"mod tests;",
			"#[cfg(test)]",
			"pub mod fixtures {",
			"}",
			"#[cfg(test)]",
		]
		.join("\n");

		assert_eq!(test_modules(&source), [5..=7]);
	}

	#[test]
	fn an_unclosed_module_runs_to_the_end_of_the_file() {
		assert_eq!(
			test_modules("#[cfg(test)]\nmod tests {\n\tfn t() {}"),
			[1..=3]
		);
	}

	#[test]
	fn recognizes_test_files() {
		assert!(is_test_file("crates/a/src/tests.rs"));
		assert!(is_test_file("crates/a/src/view/tests.rs"));
		assert!(!is_test_file("crates/a/src/contests.rs"));
		assert!(!is_test_file("crates/a/src/lib.rs"));
	}
}
