//! The release bundle report: what the browser downloads, and the budget.
//!
//! Release builds print every `.js` and `.wasm` file under `pkg/` raw and
//! compressed, and total the **initial load**: everything the page fetches
//! before any lazily loaded chunk, which is what the user waits on.
//! `bundle.budget_kib` caps the initial load's brotli size, so a pull request
//! that bloats the bundle fails its build instead of shipping.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Result;

use crate::compress;
use crate::compress::Sizes;

/// One downloaded file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
	/// The path under the site root, with `/` separators.
	pub path: String,
	/// Its sizes.
	pub sizes: Sizes,
	/// Whether it loads on demand rather than with the page.
	pub lazy: bool,
}

/// The measured client bundle.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BundleReport {
	/// Every `.js` and `.wasm` file, sorted by path.
	pub entries: Vec<Entry>,
}

impl BundleReport {
	/// Measures the scripts and wasm modules under `pkg_dir`, naming them
	/// relative to `site_root`. `is_lazy` marks the files that load on
	/// demand, and `known` holds sizes this build already computed (by
	/// precompressing), which are reused rather than compressed again.
	pub fn measure(
		site_root: &Path,
		pkg_dir: &Path,
		is_lazy: impl Fn(&Path) -> bool,
		known: &BTreeMap<PathBuf, Sizes>,
	) -> Result<Self> {
		let mut entries = Vec::new();
		for entry in walkdir::WalkDir::new(pkg_dir) {
			let entry = entry?;
			let path = entry.path();
			let downloaded = matches!(
				path.extension().and_then(|extension| extension.to_str()),
				Some("js" | "wasm")
			);
			if !entry.file_type().is_file() || !downloaded {
				continue;
			}
			let relative = path.strip_prefix(site_root).unwrap_or(path);
			entries.push(Entry {
				path: relative
					.components()
					.map(|component| component.as_os_str().to_string_lossy())
					.collect::<Vec<_>>()
					.join("/"),
				sizes: match known.get(path) {
					Some(sizes) => *sizes,
					None => compress::measure(path)?,
				},
				lazy: is_lazy(path),
			});
		}
		entries.sort_by(|left, right| (left.lazy, &left.path).cmp(&(right.lazy, &right.path)));
		Ok(Self { entries })
	}

	/// The total of every file the page loads up front.
	#[must_use]
	pub fn initial(&self) -> Sizes {
		self.entries
			.iter()
			.filter(|entry| !entry.lazy)
			.fold(Sizes::default(), |total, entry| total + entry.sizes)
	}

	/// Fails when the initial load's brotli size exceeds `budget_kib`.
	pub fn check_budget(&self, budget_kib: Option<u32>) -> Result<()> {
		let Some(budget) = budget_kib else {
			return Ok(());
		};
		let initial = self.initial().brotli;
		anyhow::ensure!(
			initial <= u64::from(budget) * 1024,
			"the initial load is {} brotli-compressed, over bundle.budget_kib = {budget}",
			kib(initial)
		);
		Ok(())
	}

	/// The report as an aligned table.
	#[must_use]
	pub fn render(&self, budget_kib: Option<u32>) -> String {
		let width = self
			.entries
			.iter()
			.map(|entry| entry.path.len() + if entry.lazy { 7 } else { 0 })
			.chain([12])
			.max()
			.unwrap_or_default();
		let mut out = String::new();
		let _ = writeln!(
			out,
			"  {:width$}  {:>10}  {:>10}  {:>10}",
			"", "raw", "gzip", "brotli"
		);
		for entry in &self.entries {
			let name = if entry.lazy {
				format!("{} (lazy)", entry.path)
			} else {
				entry.path.clone()
			};
			let _ = writeln!(out, "  {name:width$}  {}", columns(entry.sizes));
		}
		let initial = self.initial();
		let _ = write!(out, "  {:width$}  {}", "initial load", columns(initial));
		if let Some(budget) = budget_kib {
			let verdict = if initial.brotli <= u64::from(budget) * 1024 {
				"within"
			} else {
				"OVER"
			};
			let _ = write!(out, "  {verdict} the {budget} KiB budget");
		}
		out
	}
}

fn columns(sizes: Sizes) -> String {
	format!(
		"{:>10}  {:>10}  {:>10}",
		kib(sizes.raw),
		kib(sizes.gzip),
		kib(sizes.brotli)
	)
}

/// Bytes as KiB with one decimal.
#[must_use]
pub fn kib(bytes: u64) -> String {
	format!("{:.1} KiB", bytes as f64 / 1024.0)
}
