//! Citations for the sources a project typically holds.

use chrono::TimeZone;
use chrono::Utc;
use commonplace_db::PublicSource;
use commonplace_shared::cite::Style;
use commonplace_shared::cite::bibliography;
use commonplace_shared::cite::cite;
use leptoskit_db::Id;

fn source(author: Option<&str>, site: Option<&str>, published: Option<&str>) -> PublicSource {
	PublicSource {
		id: Id::parse("01900000-0000-7000-8000-000000000000").unwrap(),
		owner_id: Id::parse("01900000-0000-7000-8000-000000000000").unwrap(),
		project_id: Id::parse("01900000-0000-7000-8000-000000000000").unwrap(),
		url: "https://example.org/heat".into(),
		title: "Why cities run hot".into(),
		author: author.map(Into::into),
		site: site.map(Into::into),
		published: published.map(Into::into),
		accessed_at: Utc.with_ymd_and_hms(2026, 10, 30, 9, 0, 0).unwrap(),
	}
}

#[test]
fn apa() {
	let full = source(
		Some("Ada Mae Lovelace"),
		Some("The Climate Desk"),
		Some("2024-05-01"),
	);
	insta::assert_snapshot!(cite(&full, Style::Apa), @"Lovelace, A. M. (2024, May 1). Why cities run hot. The Climate Desk. https://example.org/heat");
	let bare = source(None, None, None);
	insta::assert_snapshot!(cite(&bare, Style::Apa), @"Why cities run hot. (n.d.). https://example.org/heat");
}

#[test]
fn mla() {
	let full = source(
		Some("Ada Lovelace"),
		Some("The Climate Desk"),
		Some("2024-09"),
	);
	insta::assert_snapshot!(cite(&full, Style::Mla), @"Lovelace, Ada. “Why cities run hot.” The Climate Desk, Sept. 2024, example.org/heat. Accessed 30 Oct. 2026.");
}

#[test]
fn bibtex() {
	let full = source(
		Some("Ada Lovelace"),
		Some("Climate & Co"),
		Some("2024-05-01"),
	);
	insta::assert_snapshot!(cite(&full, Style::Bibtex), @r"
	@online{lovelace2024,
	  author = {Lovelace, Ada},
	  title = {Why cities run hot},
	  organization = {Climate \& Co},
	  year = {2024},
	  month = {5},
	  url = {https://example.org/heat},
	  urldate = {2026-10-30},
	}
	");
}

#[test]
fn bibliographies_sort_and_keep_keys_apart() {
	let sources = [
		source(Some("Zoe Smith"), None, Some("2024")),
		source(Some("Ada Lovelace"), None, Some("2024")),
		source(Some("Ann Smith"), None, Some("2024")),
	];
	let apa = bibliography(&sources, Style::Apa);
	let order: Vec<&str> = apa
		.lines()
		.map(|line| line.split(',').next().unwrap())
		.collect();
	assert_eq!(order, ["Lovelace", "Smith", "Smith"]);

	let bibtex = bibliography(&sources, Style::Bibtex);
	assert!(bibtex.contains("@online{smith2024a,"));
	assert!(bibtex.contains("@online{smith2024b,"));
	assert!(bibtex.contains("@online{lovelace2024,"));
}
