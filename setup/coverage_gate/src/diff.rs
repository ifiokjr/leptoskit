//! Reading the added lines out of `git diff --unified=0`.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt;

/// The line numbers each changed file gained, in the new version of the file.
pub type ChangedLines = BTreeMap<String, BTreeSet<u32>>;

/// A diff line that does not follow the unified format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
	/// The 1-based line of the diff.
	pub line: usize,
	/// What was wrong with it.
	pub message: String,
}

impl fmt::Display for ParseError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "diff line {}: {}", self.line, self.message)
	}
}

impl std::error::Error for ParseError {}

/// Collects the added lines of a unified diff with the default `a/` and `b/`
/// prefixes, as `git diff --unified=0` writes it.
///
/// The line numbers come from the hunk headers: with zero context lines the
/// new-side range of a hunk is exactly the lines it added. Hunk bodies are
/// counted off rather than read, so an added line whose text begins with
/// `++ ` is not mistaken for the next file header. A deleted file
/// (`+++ /dev/null`) contributes nothing, and a hunk that only removes lines
/// has an empty new-side range.
pub fn parse(diff: &str) -> Result<ChangedLines, ParseError> {
	let mut changes = ChangedLines::new();
	let mut current: Option<String> = None;
	// Body lines left in the current hunk; `\ No newline at end of file`
	// markers are not counted by the header and are skipped separately.
	let mut body: u64 = 0;

	for (index, line) in diff.lines().enumerate() {
		let error = |message: String| {
			ParseError {
				line: index + 1,
				message,
			}
		};

		if line.starts_with('\\') {
			continue;
		}
		if body > 0 {
			if !(line.starts_with('+') || line.starts_with('-')) {
				return Err(error(format!(
					"expected {body} more hunk line(s), found `{line}`"
				)));
			}
			body -= 1;
			continue;
		}

		if let Some(target) = line.strip_prefix("+++ ") {
			current = if target == "/dev/null" {
				None
			} else {
				let path = target
					.strip_prefix("b/")
					.ok_or_else(|| error(format!("expected a `b/` path, found `{target}`")))?;
				Some(path.to_owned())
			};
		} else if let Some(header) = line.strip_prefix("@@ ") {
			let mut fields = header.split(' ');
			let old = fields
				.next()
				.and_then(|field| field.strip_prefix('-'))
				.and_then(parse_range);
			let new = fields
				.next()
				.and_then(|field| field.strip_prefix('+'))
				.and_then(parse_range);
			let (Some((_, removed)), Some((start, added))) = (old, new) else {
				return Err(error(format!("malformed hunk header `{line}`")));
			};
			body = u64::from(removed) + u64::from(added);
			if let Some(file) = &current
				&& added > 0
			{
				changes
					.entry(file.clone())
					.or_default()
					.extend(start..start + added);
			}
		}
	}

	if body > 0 {
		return Err(ParseError {
			line: diff.lines().count(),
			message: format!("the last hunk is missing {body} line(s)"),
		});
	}

	Ok(changes)
}

/// `start[,count]`, where a missing count means one line.
fn parse_range(range: &str) -> Option<(u32, u32)> {
	let (start, count) = match range.split_once(',') {
		Some((start, count)) => (start, count.parse().ok()?),
		None => (range, 1),
	};
	Some((start.parse().ok()?, count))
}

#[cfg(test)]
mod tests {
	use super::*;

	const DIFF: &str = "\
diff --git a/crates/a/src/lib.rs b/crates/a/src/lib.rs
index 1111111..2222222 100644
--- a/crates/a/src/lib.rs
+++ b/crates/a/src/lib.rs
@@ -3,0 +4,2 @@ fn existing() {
+	first();
+	second();
@@ -10 +12 @@ fn other() {
-	old();
+	new();
@@ -20,2 +21,0 @@ fn removed() {
-	gone();
-	gone();
diff --git a/old.rs b/old.rs
deleted file mode 100644
--- a/old.rs
+++ /dev/null
@@ -1,2 +0,0 @@
-fn old() {}
-fn older() {}
diff --git a/new.rs b/new.rs
new file mode 100644
--- /dev/null
+++ b/new.rs
@@ -0,0 +1,3 @@
+fn a() {}
+fn b() {}
+fn c() {}
";

	#[test]
	fn collects_the_new_side_of_every_hunk() {
		let changes = parse(DIFF).unwrap();

		assert_eq!(
			changes.get("crates/a/src/lib.rs"),
			Some(&BTreeSet::from([4, 5, 12]))
		);
		assert_eq!(changes.get("new.rs"), Some(&BTreeSet::from([1, 2, 3])));
		assert_eq!(changes.get("old.rs"), None);
		assert_eq!(changes.len(), 2);
	}

	#[test]
	fn an_empty_diff_changes_nothing() {
		assert!(parse("").unwrap().is_empty());
	}

	#[test]
	fn rejects_paths_without_the_default_prefix() {
		let error = parse("+++ crates/a/src/lib.rs\n").unwrap_err();

		assert_eq!(
			error.to_string(),
			"diff line 1: expected a `b/` path, found `crates/a/src/lib.rs`"
		);
	}

	#[test]
	fn counts_hunk_bodies_instead_of_reading_them() {
		// The added line's text is `++ b/x.rs`, and the file ends without a
		// newline; neither may be read as a header.
		let diff =
			"+++ b/a.rs\n@@ -1 +1,2 @@\n-old\n++++ b/x.rs\n+new\n\\ No newline at end of file\n";

		let changes = parse(diff).unwrap();

		assert_eq!(changes.get("a.rs"), Some(&BTreeSet::from([1, 2])));
		assert_eq!(changes.len(), 1);
	}

	#[test]
	fn rejects_malformed_hunk_headers() {
		assert_eq!(
			parse("+++ b/a.rs\n@@ -1 +x,2 @@\n").unwrap_err().message,
			"malformed hunk header `@@ -1 +x,2 @@`"
		);
		assert_eq!(
			parse("+++ b/a.rs\n@@ -1 @@\n").unwrap_err().message,
			"malformed hunk header `@@ -1 @@`"
		);
	}

	#[test]
	fn rejects_truncated_hunks() {
		assert_eq!(
			parse("+++ b/a.rs\n@@ -1 +1,2 @@\n-old\n+new\n")
				.unwrap_err()
				.message,
			"the last hunk is missing 1 line(s)"
		);
		assert_eq!(
			parse("+++ b/a.rs\n@@ -1 +1 @@\n-old\n context\n")
				.unwrap_err()
				.message,
			"expected 1 more hunk line(s), found ` context`"
		);
	}
}
