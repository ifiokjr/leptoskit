//! Citations for web pages in APA (7th edition), MLA (9th edition), and
//! BibTeX, formatted the same way on every surface.

use std::fmt::Write as _;

use chrono::Datelike;
use commonplace_db::PublicSource;
use serde::Deserialize;
use serde::Serialize;

/// A citation style.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Style {
	#[default]
	Apa,
	Mla,
	Bibtex,
}

impl Style {
	/// Every style, in the order a picker shows them.
	pub const ALL: [Self; 3] = [Self::Apa, Self::Mla, Self::Bibtex];

	/// The style's name, as people know it.
	#[must_use]
	pub const fn label(self) -> &'static str {
		match self {
			Self::Apa => "APA",
			Self::Mla => "MLA",
			Self::Bibtex => "BibTeX",
		}
	}
}

impl std::fmt::Display for Style {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter.write_str(match self {
			Self::Apa => "apa",
			Self::Mla => "mla",
			Self::Bibtex => "bibtex",
		})
	}
}

impl std::str::FromStr for Style {
	type Err = String;

	fn from_str(text: &str) -> Result<Self, Self::Err> {
		match text {
			"apa" => Ok(Self::Apa),
			"mla" => Ok(Self::Mla),
			"bibtex" => Ok(Self::Bibtex),
			_ => Err(format!("no citation style is called {text}")),
		}
	}
}

const MONTHS: [&str; 12] = [
	"January",
	"February",
	"March",
	"April",
	"May",
	"June",
	"July",
	"August",
	"September",
	"October",
	"November",
	"December",
];

/// A publication date as stored: a year, and maybe a month and a day.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Published {
	year: i32,
	month: Option<u32>,
	day: Option<u32>,
}

fn published(source: &PublicSource) -> Option<Published> {
	let text = source.published.as_deref()?;
	let mut parts = text.split('-');
	let year = parts.next()?.parse().ok()?;
	let month = parts
		.next()
		.and_then(|month| month.parse().ok())
		.filter(|month| (1..=12).contains(month));
	let day = month.and(parts.next().and_then(|day| day.parse().ok()));
	Some(Published { year, month, day })
}

fn month_name(month: u32) -> &'static str {
	MONTHS[(month - 1) as usize]
}

/// MLA abbreviates months longer than four letters.
fn mla_month(month: u32) -> String {
	let name = month_name(month);
	if name.len() > 4 && month != 9 {
		format!("{}.", &name[..3])
	} else if month == 9 {
		"Sept.".to_owned()
	} else {
		name.to_owned()
	}
}

/// "Ada Lovelace" as "Lovelace, A."; a name that is one word, such as an
/// organization, stays as it is.
fn apa_author(author: &str) -> String {
	let words: Vec<&str> = author.split_whitespace().collect();
	match words.split_last() {
		Some((family, given)) if !given.is_empty() => {
			let initials: Vec<String> = given
				.iter()
				.filter_map(|name| name.chars().next())
				.map(|initial| format!("{initial}."))
				.collect();
			format!("{family}, {}", initials.join(" "))
		}
		_ => author.trim().to_owned(),
	}
}

/// "Ada Lovelace" as "Lovelace, Ada".
fn inverted(author: &str) -> String {
	let words: Vec<&str> = author.split_whitespace().collect();
	match words.split_last() {
		Some((family, given)) if !given.is_empty() => format!("{family}, {}", given.join(" ")),
		_ => author.trim().to_owned(),
	}
}

fn sentence_end(text: &str) -> String {
	let text = text.trim();
	if text.ends_with(['.', '?', '!']) {
		text.to_owned()
	} else {
		format!("{text}.")
	}
}

/// One source in one style.
#[must_use]
pub fn cite(source: &PublicSource, style: Style) -> String {
	match style {
		Style::Apa => apa(source),
		Style::Mla => mla(source),
		Style::Bibtex => bibtex(source),
	}
}

fn apa(source: &PublicSource) -> String {
	let date = match published(source) {
		Some(Published {
			year,
			month: Some(month),
			day: Some(day),
		}) => format!("({year}, {} {day})", month_name(month)),
		Some(Published {
			year,
			month: Some(month),
			..
		}) => format!("({year}, {})", month_name(month)),
		Some(Published { year, .. }) => format!("({year})"),
		None => "(n.d.)".to_owned(),
	};
	let title = sentence_end(&source.title);
	let site = source
		.site
		.as_deref()
		.filter(|site| Some(*site) != source.author.as_deref())
		.map(|site| format!(" {}", sentence_end(site)))
		.unwrap_or_default();

	match source.author.as_deref() {
		Some(author) => {
			format!(
				"{} {date}. {title}{site} {}",
				sentence_end(&apa_author(author)),
				source.url
			)
		}
		None => format!("{title} {date}.{site} {}", source.url),
	}
}

fn mla(source: &PublicSource) -> String {
	let mut citation = String::new();
	if let Some(author) = source.author.as_deref() {
		citation.push_str(&sentence_end(&inverted(author)));
		citation.push(' ');
	}
	let _ = write!(citation, "\u{201c}{}\u{201d}", sentence_end(&source.title));
	let mut container = Vec::new();
	if let Some(site) = source.site.as_deref() {
		container.push(site.trim().to_owned());
	}
	if let Some(date) = published(source) {
		container.push(match (date.month, date.day) {
			(Some(month), Some(day)) => format!("{day} {} {}", mla_month(month), date.year),
			(Some(month), None) => format!("{} {}", mla_month(month), date.year),
			_ => date.year.to_string(),
		});
	}
	container.push(
		source
			.url
			.trim_start_matches("https://")
			.trim_start_matches("http://")
			.to_owned(),
	);
	let accessed = source.accessed_at;
	let _ = write!(
		citation,
		" {}. Accessed {} {} {}.",
		container.join(", "),
		accessed.day(),
		mla_month(accessed.month()),
		accessed.year()
	);
	citation
}

/// A BibTeX key: the author's family name, or the title's first word, and
/// the year.
fn bibtex_key(source: &PublicSource) -> String {
	let word = source
		.author
		.as_deref()
		.and_then(|author| author.split_whitespace().last())
		.or_else(|| source.title.split_whitespace().next())
		.unwrap_or("source");
	let word: String = word
		.chars()
		.filter(char::is_ascii_alphanumeric)
		.collect::<String>()
		.to_lowercase();
	let year = published(source).map_or_else(|| "nd".to_owned(), |date| date.year.to_string());
	format!("{word}{year}")
}

fn bibtex_escape(text: &str) -> String {
	let mut escaped = String::with_capacity(text.len());
	for character in text.chars() {
		if matches!(character, '{' | '}' | '&' | '%' | '$' | '#' | '_') {
			escaped.push('\\');
		}
		escaped.push(character);
	}
	escaped
}

fn bibtex(source: &PublicSource) -> String {
	let mut fields = Vec::new();
	if let Some(author) = source.author.as_deref() {
		fields.push(("author", bibtex_escape(&inverted(author))));
	}
	fields.push(("title", bibtex_escape(source.title.trim())));
	if let Some(site) = source.site.as_deref() {
		fields.push(("organization", bibtex_escape(site.trim())));
	}
	if let Some(date) = published(source) {
		fields.push(("year", date.year.to_string()));
		if let Some(month) = date.month {
			fields.push(("month", month.to_string()));
		}
	}
	fields.push(("url", source.url.clone()));
	fields.push(("urldate", source.accessed_at.format("%Y-%m-%d").to_string()));

	let mut entry = format!("@online{{{},\n", bibtex_key(source));
	for (name, value) in fields {
		let _ = writeln!(entry, "  {name} = {{{value}}},");
	}
	entry.push('}');
	entry
}

/// Every source in one style, sorted as reference lists are: by author, or
/// by title for a source without one. BibTeX keys that would repeat get a
/// letter, as `smith2024a` and `smith2024b`.
#[must_use]
pub fn bibliography(sources: &[PublicSource], style: Style) -> String {
	let mut sorted: Vec<&PublicSource> = sources.iter().collect();
	sorted.sort_by_key(|source| {
		source.author.as_deref().map_or_else(
			|| source.title.to_lowercase(),
			|author| inverted(author).to_lowercase(),
		)
	});

	let separator = if style == Style::Bibtex { "\n\n" } else { "\n" };
	let mut keys: Vec<String> = Vec::new();
	let mut entries = Vec::new();
	for source in sorted {
		let mut entry = cite(source, style);
		if style == Style::Bibtex {
			let key = bibtex_key(source);
			let repeats = keys.iter().filter(|seen| **seen == key).count();
			if repeats > 0
				|| sources
					.iter()
					.filter(|other| bibtex_key(other) == key)
					.count()
					> 1
			{
				let letter = char::from(b'a' + u8::try_from(repeats).unwrap_or(25).min(25));
				entry = entry.replacen(&format!("{{{key},"), &format!("{{{key}{letter},"), 1);
			}
			keys.push(key);
		}
		entries.push(entry);
	}
	entries.join(separator)
}
