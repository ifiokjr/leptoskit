//! What Commonplace's clients and server agree on: the bodies of API requests
//! and responses, and the formatting every surface shares, such as citations.
//!
//! Rows travel in their `Public…` form from `commonplace_db`.

pub mod cite;
pub mod tags;

use commonplace_db::Clip;
use commonplace_db::Note;
use commonplace_db::Project;
use commonplace_db::PublicClip;
use commonplace_db::PublicNote;
use commonplace_db::PublicProject;
use commonplace_db::PublicSource;
use leptoskit_db::Id;
use serde::Deserialize;
use serde::Serialize;

/// A project in the library, with how much it holds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProjectSummary {
	pub project: PublicProject,
	pub sources: u64,
	pub clips: u64,
}

/// Everything in one project.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProjectDetail {
	pub project: PublicProject,
	pub sources: Vec<PublicSource>,
	/// Newest first.
	pub clips: Vec<PublicClip>,
	/// Newest first.
	pub notes: Vec<PublicNote>,
}

impl ProjectDetail {
	/// The source a clip came from.
	#[must_use]
	pub fn source_of(&self, clip: &PublicClip) -> Option<&PublicSource> {
		self.sources
			.iter()
			.find(|source| source.id == clip.source_id)
	}

	/// The clips taken from one source, newest first.
	pub fn clips_from(&self, source: &PublicSource) -> impl Iterator<Item = &PublicClip> {
		self.clips
			.iter()
			.filter(move |clip| clip.source_id == source.id)
	}
}

/// The body of `POST /api/projects`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct NewProjectInput {
	pub title: String,
	#[serde(default)]
	pub question: Option<String>,
}

/// The page a clip comes from, as the clipper or a person describes it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PageInput {
	pub url: String,
	pub title: String,
	#[serde(default)]
	pub author: Option<String>,
	#[serde(default)]
	pub site: Option<String>,
	#[serde(default)]
	pub published: Option<String>,
}

/// The quote itself, with a little context on each side to find it again.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextQuote {
	pub exact: String,
	#[serde(default)]
	pub prefix: Option<String>,
	#[serde(default)]
	pub suffix: Option<String>,
}

/// The body of `POST /api/clips`. The server finds the page's source in the
/// project, or adds it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct NewClipInput {
	pub project: Option<Id<Project>>,
	pub page: PageInput,
	pub quote: TextQuote,
	#[serde(default)]
	pub note: Option<String>,
	#[serde(default)]
	pub tags: String,
}

/// The body of `PATCH /api/clips/{id}`: only the fields that are present
/// change.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ClipChanges {
	/// `Some(None)` clears the note; it travels as `"note": null`.
	#[serde(
		default,
		deserialize_with = "present",
		skip_serializing_if = "Option::is_none"
	)]
	pub note: Option<Option<String>>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub tags: Option<String>,
}

/// Reads a field that is present, even as `null`, as `Some`: absent fields
/// take the `default`, `None`.
fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
	D: serde::Deserializer<'de>,
	T: Deserialize<'de>,
{
	Option::<T>::deserialize(deserializer).map(Some)
}

/// The body of `POST /api/projects/{id}/notes` and `PATCH /api/notes/{id}`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct NoteInput {
	pub body: String,
}

/// A clip with the page it came from, as the extension shows clips from the
/// page that is open.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClipOnPage {
	pub clip: PublicClip,
	pub source: PublicSource,
	pub project: PublicProject,
}

/// An API failure: a code to match on, and a sentence for people.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiError {
	pub error: String,
	pub message: String,
}

/// Keeps the typed IDs in one place for callers that only see this crate.
pub type ProjectId = Id<Project>;
/// A clip's ID.
pub type ClipId = Id<Clip>;
/// A note's ID.
pub type NoteId = Id<Note>;

/// The longest context kept on each side of a quote.
pub const CONTEXT_CHARS: usize = 32;

impl TextQuote {
	/// A quote with up to [`CONTEXT_CHARS`] of context on each side, taken
	/// from the text around it.
	#[must_use]
	pub fn within(before: &str, exact: &str, after: &str) -> Self {
		let prefix: String = {
			let chars: Vec<char> = before.chars().collect();
			chars[chars.len().saturating_sub(CONTEXT_CHARS)..]
				.iter()
				.collect()
		};
		let suffix: String = after.chars().take(CONTEXT_CHARS).collect();
		let non_empty = |text: String| (!text.trim().is_empty()).then_some(text);

		Self {
			exact: exact.trim().to_owned(),
			prefix: non_empty(prefix),
			suffix: non_empty(suffix),
		}
	}

	/// Where the quote is in `text`, as a byte range. The context picks the
	/// right one when the quote appears more than once.
	#[must_use]
	pub fn locate(&self, text: &str) -> Option<std::ops::Range<usize>> {
		if self.exact.is_empty() {
			return None;
		}

		let mut best: Option<(usize, usize)> = None;
		for (start, _) in text.match_indices(self.exact.as_str()) {
			let end = start + self.exact.len();
			let mut score = 0;
			if let Some(prefix) = &self.prefix
				&& text[..start].ends_with(prefix.as_str())
			{
				score += 1;
			}
			if let Some(suffix) = &self.suffix
				&& text[end..].starts_with(suffix.as_str())
			{
				score += 1;
			}
			if best.is_none_or(|(_, best_score)| score > best_score) {
				best = Some((start, score));
			}
		}
		best.map(|(start, _)| start..start + self.exact.len())
	}
}

#[cfg(test)]
mod tests {
	use super::TextQuote;

	#[test]
	fn context_picks_the_right_occurrence() {
		let text = "Heat rises. In cities, heat rises faster than in fields.";
		let quote = TextQuote::within("In cities, ", "heat rises", " faster");
		assert_eq!(quote.locate(text), Some(23..33));
	}

	#[test]
	fn context_is_trimmed_to_its_limit() {
		let before = "a".repeat(100);
		let quote = TextQuote::within(&before, " the quote ", "");
		assert_eq!(quote.exact, "the quote");
		assert_eq!(quote.prefix.unwrap().len(), super::CONTEXT_CHARS);
		assert_eq!(quote.suffix, None);
	}

	#[test]
	fn a_missing_quote_is_not_found() {
		let quote = TextQuote::within("", "absent", "");
		assert_eq!(quote.locate("present"), None);
	}
}
