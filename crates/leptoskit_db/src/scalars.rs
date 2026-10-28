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
