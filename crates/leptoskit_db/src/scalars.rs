//! Scalars most apps need, declared the same way an app declares its own.

use crate::Scalar;

/// An email address, trimmed and lowercased.
///
/// The check is deliberately pragmatic: one `@`, no spaces, and a dot in the
/// domain. Only sending a message proves an address works; this keeps out
/// typos and junk without rejecting real addresses that a full RFC 5322
/// parser would argue about.
#[derive(Scalar, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[db(
	max_len = 254,
	regex = r"^[^@\s]+@[^@\s]+\.[^@\s]+$",
	normalize(trim, lowercase)
)]
pub struct Email(String);

/// An `http` or `https` URL, trimmed.
#[derive(Scalar, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[db(max_len = 2048, regex = r"^https?://[^\s/?#]+[^\s]*$", normalize(trim))]
pub struct Url(String);

/// A URL path segment: lowercase letters and digits in words joined by
/// single hyphens, such as `hello-world-2`.
#[derive(Scalar, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[db(
	min_len = 1,
	max_len = 100,
	regex = r"^[a-z0-9]+(-[a-z0-9]+)*$",
	normalize(trim, lowercase)
)]
pub struct Slug(String);

impl Slug {
	/// The longest slug [`Slug::from_text`] makes, leaving room for the
	/// suffix that keeps a `#[db(slug(from = …))]` column unique.
	pub const MAX_MADE_LEN: usize = 80;

	/// The slug for some text, such as a title: its ASCII letters and digits,
	/// lowercased, with every run of anything else as one hyphen, cut to
	/// [`Slug::MAX_MADE_LEN`] characters at a hyphen when it can be. Text
	/// with no ASCII letter or digit, such as `"日本"`, becomes `untitled`.
	///
	/// ```
	/// # use leptoskit_db::Slug;
	/// assert_eq!(Slug::from_text("Hello, World!").as_inner(), "hello-world");
	/// assert_eq!(Slug::from_text("¿?").as_inner(), "untitled");
	/// ```
	pub fn from_text(text: &str) -> Self {
		let mut slug = String::new();

		for word in text
			.split(|character: char| !character.is_ascii_alphanumeric())
			.filter(|word| !word.is_empty())
		{
			let separator = usize::from(!slug.is_empty());

			if slug.len() + separator + word.len() > Self::MAX_MADE_LEN {
				// A first word longer than the limit is cut; later ones end
				// the slug at the hyphen before them.
				if slug.is_empty() {
					slug.push_str(&word[..Self::MAX_MADE_LEN]);
				}

				break;
			}

			if separator == 1 {
				slug.push('-');
			}

			slug.push_str(word);
		}

		slug.make_ascii_lowercase();

		if slug.is_empty() {
			slug.push_str("untitled");
		}

		Self(slug)
	}
}

// The checks need `regex`, which `server` turns on.
#[cfg(all(test, feature = "regex"))]
mod tests {
	use super::*;

	#[test]
	fn emails_are_normalized_then_checked() {
		let email = Email::parse("  Ada@Example.COM ").unwrap();
		assert_eq!(email.as_inner(), "ada@example.com");
		assert_eq!(email.to_string(), "ada@example.com");
		assert_eq!(&*email, "ada@example.com");

		for invalid in ["", "ada", "ada@example", "a da@example.com", "@example.com"] {
			assert!(Email::parse(invalid).is_err(), "{invalid:?} parsed");
		}

		let too_long = format!("{}@example.com", "a".repeat(250));
		let errors = Email::parse(too_long).unwrap_err();
		assert_eq!(
			errors.to_string(),
			"`Email` can have at most 254 characters"
		);
	}

	#[test]
	fn urls_and_slugs_check_their_shape() {
		assert_eq!(
			Url::parse(" https://example.com/a?b ")
				.unwrap()
				.into_inner(),
			"https://example.com/a?b"
		);
		assert!(Url::parse("ftp://example.com").is_err());
		assert!(Url::parse("https://").is_err());

		assert_eq!(
			Slug::parse("Hello-World-2").unwrap().as_ref(),
			"hello-world-2"
		);

		for invalid in ["", "-a", "a-", "a--b", "a b", "a_b"] {
			assert!(Slug::parse(invalid).is_err(), "{invalid:?} parsed");
		}
	}

	#[test]
	fn slugs_are_made_from_any_text() {
		let made = |text: &str| Slug::from_text(text).into_inner();

		assert_eq!(made("  Hello,   World! 2 "), "hello-world-2");
		assert_eq!(made("already-a-slug"), "already-a-slug");
		assert_eq!(made("Ünïcode ñ"), "n-code");
		assert_eq!(made(""), "untitled");
		assert_eq!(made("日本"), "untitled");

		let long = made(&format!("{} tail", "a".repeat(Slug::MAX_MADE_LEN + 5)));
		assert_eq!(long, "a".repeat(Slug::MAX_MADE_LEN));

		let words = made(&"word ".repeat(40));
		assert!(words.len() <= Slug::MAX_MADE_LEN, "{words}");
		assert!(!words.ends_with('-'), "{words}");

		for text in ["A b", "x--y", "  ", "99 Red Balloons", "ÆØÅ ok"] {
			assert!(Slug::parse(made(text)).is_ok(), "{text:?}");
		}
	}

	#[test]
	fn scalars_deserialize_through_parse() {
		let email: Email = serde_json::from_str("\"ADA@example.com\"").unwrap();
		assert_eq!(email.as_inner(), "ada@example.com");
		assert_eq!(
			serde_json::to_string(&email).unwrap(),
			"\"ada@example.com\""
		);

		let error = serde_json::from_str::<Email>("\"not an email\"").unwrap_err();
		assert!(error.to_string().contains("`Email` must match"), "{error}");

		let parsed: Slug = "Rust".parse().unwrap();
		assert_eq!(String::from(parsed), "rust");
		assert_eq!(
			Slug::try_from(String::from("a-b")).unwrap().as_inner(),
			"a-b"
		);
	}

	#[test]
	fn scalars_describe_their_rules_to_generated_clients() {
		let schema = serde_json::to_value(schemars::schema_for!(Email)).unwrap();
		assert_eq!(schema["type"], "string");
		assert_eq!(schema["maxLength"], 254);
		assert_eq!(schema["pattern"], r"^[^@\s]+@[^@\s]+\.[^@\s]+$");
		assert_eq!(schema["title"], "Email");
	}
}
