//! Reading the line records of an lcov tracefile.

use std::collections::BTreeMap;
use std::fmt;

/// Hit counts for every instrumented line, keyed by repository-relative path
/// and then by line number.
///
/// A file can appear in more than one record when several binaries compiled
/// it; the records are merged by summing the hits of each line, so a line is
/// covered when any binary executed it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Coverage {
	files: BTreeMap<String, BTreeMap<u32, u64>>,
}

impl Coverage {
	/// The instrumented lines of `path`, or `None` when no record names it.
	pub fn file(&self, path: &str) -> Option<&BTreeMap<u32, u64>> {
		self.files.get(path)
	}

	/// Every file with at least one record, in path order.
	pub fn files(&self) -> impl Iterator<Item = (&str, &BTreeMap<u32, u64>)> {
		self.files
			.iter()
			.map(|(path, lines)| (path.as_str(), lines))
	}
}

/// A tracefile line that is not valid lcov.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
	/// The 1-based line of the tracefile.
	pub line: usize,
	/// What was wrong with it.
	pub message: String,
}

impl fmt::Display for ParseError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "lcov line {}: {}", self.line, self.message)
	}
}

impl std::error::Error for ParseError {}

/// Parses the `SF`, `DA`, and `end_of_record` entries of an lcov tracefile.
///
/// Source paths are made relative to `root`, which is the repository root in
/// the same spelling the coverage tool used. Records for files outside `root`
/// (the standard library, registry dependencies) are skipped rather than
/// rejected, because they cannot be part of a change to this repository.
/// Backslashes are read as path separators so a tracefile written on Windows
/// keys files the way `git diff` names them.
pub fn parse(input: &str, root: &str) -> Result<Coverage, ParseError> {
	let root = normalize(root);
	let root = root.trim_end_matches('/');
	let mut coverage = Coverage::default();
	// `Some(None)` is an open record for a file outside `root`.
	let mut current: Option<Option<String>> = None;

	for (index, raw) in input.lines().enumerate() {
		let line = index + 1;
		let entry = raw.trim();
		let error = |message: String| ParseError { line, message };

		if let Some(path) = entry.strip_prefix("SF:") {
			if current.is_some() {
				return Err(error("`SF` before the previous record ended".into()));
			}
			let path = normalize(path);
			let relative = path
				.strip_prefix(root)
				.and_then(|rest| rest.strip_prefix('/'))
				.map(str::to_owned);
			if let Some(relative) = &relative {
				coverage.files.entry(relative.clone()).or_default();
			}
			current = Some(relative);
		} else if let Some(record) = entry.strip_prefix("DA:") {
			let Some(file) = &current else {
				return Err(error("`DA` outside a record".into()));
			};
			let mut fields = record.split(',');
			let number = fields.next().and_then(|field| field.parse::<u32>().ok());
			let hits = fields.next().and_then(|field| field.parse::<u64>().ok());
			let (Some(number), Some(hits)) = (number, hits) else {
				return Err(error(format!("malformed `DA:{record}`")));
			};
			if let Some(file) = file {
				let lines = coverage.files.entry(file.clone()).or_default();
				let total = lines.entry(number).or_default();
				*total = total.saturating_add(hits);
			}
		} else if entry == "end_of_record" && current.take().is_none() {
			return Err(error("`end_of_record` outside a record".into()));
		}
	}

	if current.is_some() {
		return Err(ParseError {
			line: input.lines().count(),
			message: "the last record has no `end_of_record`".into(),
		});
	}

	Ok(coverage)
}

fn normalize(path: &str) -> String {
	path.replace('\\', "/")
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn reads_relative_paths_and_hits() {
		let tracefile = [
			"TN:",
			"SF:/repo/crates/a/src/lib.rs",
			"FN:1,f",
			"DA:1,3",
			"DA:2,0",
			"LF:2",
			"LH:1",
			"end_of_record",
		]
		.join("\n");
		let coverage = parse(&tracefile, "/repo").unwrap();

		let lines = coverage.file("crates/a/src/lib.rs").unwrap();
		assert_eq!(lines.get(&1), Some(&3));
		assert_eq!(lines.get(&2), Some(&0));
		assert_eq!(coverage.files().count(), 1);
	}

	#[test]
	fn merges_repeated_records_by_summing_hits() {
		let coverage = parse(
			"SF:/repo/a.rs\nDA:1,0\nDA:2,1\nend_of_record\nSF:/repo/a.rs\nDA:1,2\nend_of_record\n",
			"/repo/",
		)
		.unwrap();

		let lines = coverage.file("a.rs").unwrap();
		assert_eq!(lines.get(&1), Some(&2));
		assert_eq!(lines.get(&2), Some(&1));
	}

	#[test]
	fn skips_files_outside_the_root() {
		let tracefile = [
			"SF:/rustc/abc/library/core/src/lib.rs",
			"DA:1,1",
			"end_of_record",
			"SF:/repository/a.rs",
			"DA:1,1",
			"end_of_record",
		]
		.join("\n");
		let coverage = parse(&tracefile, "/repo").unwrap();

		assert_eq!(coverage.files().count(), 0);
	}

	#[test]
	fn reads_windows_separators() {
		let coverage = parse(
			"SF:C:\\repo\\crates\\a\\src\\lib.rs\nDA:4,1,checksum\nend_of_record\n",
			"C:\\repo",
		)
		.unwrap();

		assert_eq!(
			coverage.file("crates/a/src/lib.rs").unwrap().get(&4),
			Some(&1)
		);
	}

	#[test]
	fn keeps_files_without_instrumented_lines() {
		let coverage = parse("SF:/repo/a.rs\nend_of_record\n", "/repo").unwrap();

		assert!(coverage.file("a.rs").unwrap().is_empty());
	}

	#[test]
	fn rejects_malformed_line_records() {
		let error = parse("SF:/repo/a.rs\nDA:one,1\nend_of_record\n", "/repo").unwrap_err();

		assert_eq!(error.line, 2);
		assert_eq!(error.to_string(), "lcov line 2: malformed `DA:one,1`");
	}

	#[test]
	fn rejects_records_that_never_open_or_close() {
		assert_eq!(parse("DA:1,1\n", "/repo").unwrap_err().line, 1);
		assert_eq!(parse("end_of_record\n", "/repo").unwrap_err().line, 1);
		assert_eq!(
			parse("SF:/repo/a.rs\nSF:/repo/b.rs\n", "/repo")
				.unwrap_err()
				.line,
			2
		);
		assert_eq!(
			parse("SF:/repo/a.rs\nDA:1,1\n", "/repo")
				.unwrap_err()
				.message,
			"the last record has no `end_of_record`"
		);
	}
}
