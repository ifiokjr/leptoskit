//! The coverage policy: what is measured, how it is summarized, and when it
//! fails.

use std::collections::BTreeMap;
use std::fmt;
use std::fmt::Write as _;
use std::path::Path;
use std::str::FromStr;

use crate::diff::ChangedLines;
use crate::lcov::Coverage;

/// A percentage with two decimal places, held as hundredths of a percent so
/// that comparisons against the floor are exact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Percent(u32);

impl Percent {
	/// The share `covered / total`, rounded down so a value never reads as
	/// meeting a threshold it misses. `None` when there is nothing to measure.
	pub fn of(covered: u64, total: u64) -> Option<Self> {
		(total > 0).then(|| Self((covered.min(total) * 10_000 / total) as u32))
	}

	/// The value without the percent sign, as the floor file stores it:
	/// `72.41`.
	pub fn to_decimal(self) -> String {
		format!("{}.{:02}", self.0 / 100, self.0 % 100)
	}
}

impl fmt::Display for Percent {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}%", self.to_decimal())
	}
}

impl FromStr for Percent {
	type Err = String;

	/// Reads `90`, `90.5`, or `90.25`; at most two decimal places and at most
	/// `100`.
	fn from_str(value: &str) -> Result<Self, Self::Err> {
		let invalid = || format!("`{value}` is not a percentage with at most two decimals");
		let (whole, fraction) = value.trim().split_once('.').unwrap_or((value.trim(), ""));
		if whole.is_empty() || fraction.len() > 2 || !fraction.bytes().all(|b| b.is_ascii_digit()) {
			return Err(invalid());
		}
		let whole: u32 = whole.parse().map_err(|_| invalid())?;
		let fraction: u32 = format!("{fraction:0<2}").parse().map_err(|_| invalid())?;
		let hundredths = whole
			.checked_mul(100)
			.and_then(|value| value.checked_add(fraction))
			.filter(|value| *value <= 10_000)
			.ok_or_else(invalid)?;
		Ok(Self(hundredths))
	}
}

/// Covered and instrumented line counts.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Lines {
	/// Instrumented lines that ran at least once.
	pub covered: u64,
	/// Instrumented lines.
	pub total: u64,
}

impl Lines {
	/// The covered share, or `None` when nothing is instrumented.
	pub fn percent(self) -> Option<Percent> {
		Percent::of(self.covered, self.total)
	}

	fn add(&mut self, hits: u64) {
		self.total += 1;
		if hits > 0 {
			self.covered += 1;
		}
	}
}

/// The thresholds a report is held to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
	/// The workspace total may not drop below this.
	pub floor: Percent,
	/// The changed lines of a pull request must be covered at least this much.
	pub patch_minimum: Percent,
}

/// Whether a repository path counts toward coverage.
///
/// Test code is excluded, since it does the covering and counting its own
/// lines would inflate the result: `tests/` and `benches/` directories,
/// `tests.rs` modules, the
/// `examples/` targets inside a package (verifiers and journey harnesses), and
/// the benchmark harness under `benchmarks/`. Generated code under `target/`
/// (build-script output) is not source and is excluded too. The example apps
/// under the top-level `examples/` are workspace packages with their own tests
/// and are measured like any other.
pub fn is_measured(path: &str) -> bool {
	Path::new(path)
		.extension()
		.is_some_and(|extension| extension == "rs")
		&& !path.starts_with("benchmarks/")
		&& !path.starts_with("target/")
		&& !crate::source::is_test_file(path)
		&& !path
			.split('/')
			.skip(1)
			.any(|component| matches!(component, "tests" | "benches" | "examples"))
}

/// The coverage of one workspace package directory, such as
/// `crates/leptoskit_cli`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
	/// The directory, relative to the repository root.
	pub path: String,
	/// Its measured lines.
	pub lines: Lines,
}

/// The coverage of a change.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Patch {
	/// The changed lines that are instrumented.
	pub lines: Lines,
	/// Uncovered changed lines per file, as inclusive ranges.
	pub uncovered: BTreeMap<String, Vec<(u32, u32)>>,
	/// Changed measured files without a coverage record, with the number of
	/// changed lines in each. A file has no record when it holds no executable
	/// code (only `mod` declarations or types) or when no instrumented binary
	/// compiled it, as with a module for another platform. Its lines cannot be
	/// classified, so they are reported rather than counted.
	pub unmeasured: BTreeMap<String, usize>,
}

/// The summarized coverage of a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
	/// Every package directory with measured lines, in path order.
	pub packages: Vec<Package>,
	/// The measured lines of the whole workspace.
	pub total: Lines,
	/// The coverage of the change, when a base was given.
	pub patch: Option<Patch>,
}

impl Report {
	/// Summarizes `coverage`, and the coverage of `changes` when given.
	/// Paths under an `excluded` prefix count neither way: they hold code no
	/// measuring platform can run, like platform-tool glue.
	pub fn new(coverage: &Coverage, changes: Option<&ChangedLines>, excluded: &[String]) -> Self {
		let measured = |path: &str| {
			is_measured(path)
				&& !excluded
					.iter()
					.any(|prefix| path.starts_with(prefix.as_str()))
		};
		let mut packages: BTreeMap<String, Lines> = BTreeMap::new();
		let mut total = Lines::default();
		for (path, lines) in coverage.files().filter(|(path, _)| measured(path)) {
			let package = packages.entry(package_of(path)).or_default();
			for hits in lines.values() {
				package.add(*hits);
				total.add(*hits);
			}
		}

		let patch = changes.map(|changes| {
			let mut patch = Patch::default();
			for (path, changed) in changes.iter().filter(|(path, _)| measured(path)) {
				let Some(lines) = coverage.file(path) else {
					patch.unmeasured.insert(path.clone(), changed.len());
					continue;
				};
				let mut uncovered: Vec<(u32, u32)> = Vec::new();
				for (number, hits) in changed
					.iter()
					.filter_map(|number| lines.get(number).map(|hits| (*number, *hits)))
				{
					patch.lines.add(hits);
					if hits > 0 {
						continue;
					}
					// Uncovered lines separated only by uninstrumented ones
					// (blank lines, comments, signatures) read as one range.
					match uncovered.last_mut() {
						Some((_, end)) if lines.range(*end + 1..number).next().is_none() => {
							*end = number;
						}
						_ => uncovered.push((number, number)),
					}
				}
				if !uncovered.is_empty() {
					patch.uncovered.insert(path.clone(), uncovered);
				}
			}
			patch
		});

		Self {
			packages: packages
				.into_iter()
				.map(|(path, lines)| Package { path, lines })
				.collect(),
			total,
			patch,
		}
	}

	/// Every way this report misses `policy`.
	pub fn failures(&self, policy: Policy) -> Vec<Failure> {
		let mut failures = Vec::new();
		match self.total.percent() {
			None => failures.push(Failure::NothingMeasured),
			Some(total) if total < policy.floor => {
				failures.push(Failure::BelowFloor {
					total,
					floor: policy.floor,
				});
			}
			Some(_) => {}
		}
		if let Some(patch) = self.patch.as_ref().and_then(|patch| patch.lines.percent()) {
			if patch < policy.patch_minimum {
				failures.push(Failure::PatchBelowMinimum {
					patch,
					minimum: policy.patch_minimum,
				});
			}
		}
		failures
	}

	/// The report as GitHub-flavored markdown, for a job summary.
	pub fn markdown(&self, policy: Policy) -> String {
		let mut out = String::from("## Coverage\n\n");
		out.push_str("| Package | Lines | Covered |\n| --- | ---: | ---: |\n");
		for package in &self.packages {
			let _ = writeln!(
				out,
				"| `{}` | {} | {} |",
				package.path,
				package.lines.total,
				display(package.lines.percent())
			);
		}
		let _ = writeln!(
			out,
			"| **Total** | {} | **{}** (floor {}) |",
			self.total.total,
			display(self.total.percent()),
			policy.floor
		);

		if let Some(patch) = &self.patch {
			out.push_str("\n### Patch coverage\n\n");
			match patch.lines.percent() {
				Some(percent) => {
					let _ = writeln!(
						out,
						"**{percent}** of {} changed instrumented lines are covered (minimum {}).",
						patch.lines.total, policy.patch_minimum
					);
				}
				None => out.push_str("The change has no instrumented lines.\n"),
			}
			if !patch.uncovered.is_empty() {
				out.push_str("\nUncovered changed lines:\n\n");
				for (path, ranges) in &patch.uncovered {
					let ranges: Vec<String> =
						ranges.iter().map(|range| format_range(*range)).collect();
					let _ = writeln!(out, "- `{path}`: {}", ranges.join(", "));
				}
			}
			if !patch.unmeasured.is_empty() {
				out.push_str(
					"\nChanged files without a coverage record, because they hold no executable \
					 code or were not compiled on this runner (not counted):\n\n",
				);
				for (path, count) in &patch.unmeasured {
					let _ = writeln!(out, "- `{path}` ({count} changed lines)");
				}
			}
		}

		let failures = self.failures(policy);
		out.push('\n');
		if failures.is_empty() {
			out.push_str("**Result:** passed.\n");
		} else {
			for failure in failures {
				let _ = writeln!(out, "**Failed:** {failure}.");
			}
		}
		out
	}

	/// GitHub workflow commands that mark each uncovered changed range on the
	/// pull request diff.
	pub fn annotations(&self) -> String {
		let mut out = String::new();
		for (path, ranges) in self.patch.iter().flat_map(|patch| &patch.uncovered) {
			for (start, end) in ranges {
				let _ = writeln!(
					out,
					"::warning file={path},line={start},endLine={end},title=Uncovered change::{} \
					 not covered by any test",
					if start == end {
						format!("Line {start} is")
					} else {
						format!("Lines {start}-{end} are")
					}
				);
			}
		}
		out
	}
}

/// A way a report misses its policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
	/// No measured line was instrumented, so the run measured nothing.
	NothingMeasured,
	/// The workspace total fell below the floor.
	BelowFloor { total: Percent, floor: Percent },
	/// The change is not covered enough.
	PatchBelowMinimum { patch: Percent, minimum: Percent },
}

impl fmt::Display for Failure {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::NothingMeasured => write!(f, "the tracefile has no measured lines"),
			Self::BelowFloor { total, floor } => {
				write!(
					f,
					"workspace coverage {total} is below the floor of {floor}"
				)
			}
			Self::PatchBelowMinimum { patch, minimum } => {
				write!(
					f,
					"patch coverage {patch} is below the minimum of {minimum}"
				)
			}
		}
	}
}

/// The package directory of a path: its first two components, such as
/// `crates/leptoskit_cli`, or the file itself at the top level.
fn package_of(path: &str) -> String {
	let mut components = path.splitn(3, '/');
	match (components.next(), components.next(), components.next()) {
		(Some(first), Some(second), Some(_)) => format!("{first}/{second}"),
		(Some(first), Some(_), None) => first.to_owned(),
		_ => path.to_owned(),
	}
}

fn display(percent: Option<Percent>) -> String {
	percent.map_or_else(|| "n/a".to_owned(), |percent| percent.to_string())
}

fn format_range((start, end): (u32, u32)) -> String {
	if start == end {
		start.to_string()
	} else {
		format!("{start}–{end}")
	}
}

#[cfg(test)]
mod tests {
	use std::collections::BTreeSet;

	use super::*;
	use crate::lcov;

	const POLICY: Policy = Policy {
		floor: Percent(5_000),
		patch_minimum: Percent(9_000),
	};

	fn coverage() -> Coverage {
		let tracefile = [
			"SF:/repo/crates/a/src/lib.rs",
			"DA:1,1",
			"DA:2,0",
			"DA:3,0",
			"DA:5,0",
			"DA:6,4",
			"DA:7,0",
			"end_of_record",
			"SF:/repo/crates/a/tests/it.rs",
			"DA:1,0",
			"end_of_record",
			"SF:/repo/crates/b/src/lib.rs",
			"DA:1,1",
			"DA:2,1",
			"end_of_record",
			"SF:/repo/build.rs",
			"DA:1,1",
			"end_of_record",
		]
		.join("\n");
		lcov::parse(&tracefile, "/repo").unwrap()
	}

	fn changes(entries: &[(&str, &[u32])]) -> ChangedLines {
		entries
			.iter()
			.map(|(path, lines)| {
				(
					(*path).to_owned(),
					lines.iter().copied().collect::<BTreeSet<_>>(),
				)
			})
			.collect()
	}

	#[test]
	fn percent_parses_and_rounds_down() {
		assert_eq!("90".parse(), Ok(Percent(9_000)));
		assert_eq!("90.5".parse(), Ok(Percent(9_050)));
		assert_eq!(" 72.41\n".parse(), Ok(Percent(7_241)));
		assert_eq!("100.00".parse(), Ok(Percent(10_000)));
		for invalid in [
			"",
			".5",
			"90.123",
			"100.01",
			"-1",
			"9a",
			"90.x",
			"4294967296",
		] {
			assert!(invalid.parse::<Percent>().is_err(), "{invalid:?} parsed");
		}

		assert_eq!(Percent::of(2, 3), Some(Percent(6_666)));
		assert_eq!(Percent::of(5, 3), Some(Percent(10_000)));
		assert_eq!(Percent::of(0, 0), None);
		assert_eq!(Percent(7_205).to_string(), "72.05%");
	}

	#[test]
	fn measures_source_and_example_apps_but_not_test_code() {
		assert!(is_measured("crates/a/src/lib.rs"));
		assert!(is_measured("build.rs"));
		assert!(!is_measured("crates/a/tests/it.rs"));
		assert!(!is_measured("crates/a/benches/speed.rs"));
		assert!(!is_measured("crates/a/examples/demo.rs"));
		assert!(is_measured("examples/gallery/src/lib.rs"));
		assert!(!is_measured("examples/gallery/tests/desktop.rs"));
		assert!(!is_measured("examples/gallery/examples/snapshot.rs"));
		assert!(!is_measured("benchmarks/native/src/main.rs"));
		assert!(!is_measured("target/debug/build/a-1234/out/bindings.rs"));
		assert!(!is_measured("crates/a/src/view.swift"));
	}

	#[test]
	fn groups_paths_by_package_directory() {
		assert_eq!(package_of("crates/a/src/lib.rs"), "crates/a");
		assert_eq!(package_of("setup/tool.rs"), "setup");
		assert_eq!(package_of("build.rs"), "build.rs");
	}

	#[test]
	fn summarizes_packages_and_the_total() {
		let report = Report::new(&coverage(), None, &[]);

		assert_eq!(
			report.packages,
			[
				Package {
					path: "build.rs".into(),
					lines: Lines {
						covered: 1,
						total: 1
					}
				},
				Package {
					path: "crates/a".into(),
					lines: Lines {
						covered: 2,
						total: 6
					}
				},
				Package {
					path: "crates/b".into(),
					lines: Lines {
						covered: 2,
						total: 2
					}
				},
			]
		);
		assert_eq!(
			report.total,
			Lines {
				covered: 5,
				total: 9
			}
		);
		assert_eq!(report.patch, None);
		assert!(report.failures(POLICY).is_empty());
	}

	#[test]
	fn classifies_changed_lines() {
		let changes = changes(&[
			// 4 is not instrumented; 2, 3, and 5 merge across it into one
			// range, and 7 stands alone after the covered 6.
			("crates/a/src/lib.rs", &[1, 2, 3, 4, 5, 6, 7]),
			("crates/a/tests/it.rs", &[1]),
			("crates/c/src/windows.rs", &[1, 2, 3]),
			("readme.md", &[1]),
		]);

		let patch = Report::new(&coverage(), Some(&changes), &[]).patch.unwrap();

		assert_eq!(
			patch.lines,
			Lines {
				covered: 2,
				total: 6
			}
		);
		assert_eq!(
			patch.uncovered,
			BTreeMap::from([("crates/a/src/lib.rs".into(), vec![(2, 5), (7, 7)])])
		);
		assert_eq!(
			patch.unmeasured,
			BTreeMap::from([("crates/c/src/windows.rs".into(), 3)])
		);
	}

	#[test]
	fn fails_below_the_floor_and_the_patch_minimum() {
		let changes = changes(&[("crates/a/src/lib.rs", &[2, 6])]);
		let report = Report::new(&coverage(), Some(&changes), &[]);
		let strict = Policy {
			floor: Percent(5_600),
			..POLICY
		};

		assert_eq!(
			report.failures(strict),
			[
				Failure::BelowFloor {
					total: Percent(5_555),
					floor: Percent(5_600)
				},
				Failure::PatchBelowMinimum {
					patch: Percent(5_000),
					minimum: Percent(9_000)
				},
			]
		);
		assert_eq!(
			report.failures(strict)[0].to_string(),
			"workspace coverage 55.55% is below the floor of 56.00%"
		);
		assert_eq!(
			report.failures(strict)[1].to_string(),
			"patch coverage 50.00% is below the minimum of 90.00%"
		);
	}

	#[test]
	fn a_change_without_instrumented_lines_passes() {
		let changes = changes(&[("crates/a/src/lib.rs", &[4]), ("readme.md", &[1])]);
		let report = Report::new(&coverage(), Some(&changes), &[]);

		assert_eq!(report.patch.as_ref().unwrap().lines, Lines::default());
		assert!(report.failures(POLICY).is_empty());
		assert!(
			report
				.markdown(POLICY)
				.contains("The change has no instrumented lines.")
		);
	}

	#[test]
	fn an_empty_tracefile_fails() {
		let report = Report::new(&Coverage::default(), None, &[]);

		assert_eq!(report.failures(POLICY), [Failure::NothingMeasured]);
		assert_eq!(
			Failure::NothingMeasured.to_string(),
			"the tracefile has no measured lines"
		);
		assert!(
			report
				.markdown(POLICY)
				.contains("| **Total** | 0 | **n/a** (floor 50.00%) |")
		);
	}

	#[test]
	fn renders_markdown_and_annotations() {
		let changes = changes(&[
			("crates/a/src/lib.rs", &[2, 3, 6, 7]),
			("crates/c/src/windows.rs", &[1]),
		]);
		let report = Report::new(&coverage(), Some(&changes), &[]);

		assert_eq!(
			report.markdown(POLICY),
			"## Coverage

| Package | Lines | Covered |
| --- | ---: | ---: |
| `build.rs` | 1 | 100.00% |
| `crates/a` | 6 | 33.33% |
| `crates/b` | 2 | 100.00% |
| **Total** | 9 | **55.55%** (floor 50.00%) |

### Patch coverage

**25.00%** of 4 changed instrumented lines are covered (minimum 90.00%).

Uncovered changed lines:

- `crates/a/src/lib.rs`: 2–3, 7

Changed files without a coverage record, because they hold no executable code or were not compiled \
			 on this runner (not counted):

- `crates/c/src/windows.rs` (1 changed lines)

**Failed:** patch coverage 25.00% is below the minimum of 90.00%.
"
		);
		assert_eq!(
			report.annotations(),
			"::warning file=crates/a/src/lib.rs,line=2,endLine=3,title=Uncovered change::Lines \
			 2-3 are not covered by any test\n::warning \
			 file=crates/a/src/lib.rs,line=7,endLine=7,title=Uncovered change::Line 7 is not \
			 covered by any test\n"
		);
		assert!(
			Report::new(&coverage(), None, &[])
				.markdown(POLICY)
				.ends_with("**Result:** passed.\n")
		);
	}
	#[test]
	fn excluded_prefixes_count_neither_way() {
		let changed = changes(&[
			("crates/a/src/lib.rs", &[1, 2]),
			("crates/glu/src/lib.rs", &[1, 2, 3]),
		]);
		// Without the exclusion the glue file's lines report as unmeasured.
		let plain = Report::new(&coverage(), Some(&changed), &[]);
		assert!(
			plain
				.patch
				.as_ref()
				.unwrap()
				.unmeasured
				.contains_key("crates/glu/src/lib.rs")
		);
		// With it, the file counts neither way.
		let excluded = Report::new(&coverage(), Some(&changed), &["crates/glu".to_owned()]);
		let patch = excluded.patch.as_ref().unwrap();
		assert!(!patch.unmeasured.contains_key("crates/glu/src/lib.rs"));
	}
}
