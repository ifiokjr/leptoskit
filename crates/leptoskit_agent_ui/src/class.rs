//! Which Tailwind classes a document may use.
//!
//! A document styles its own content and nothing else. Classes that take an
//! element out of the flow could lay agent content over the application's
//! own interface, such as a fake confirmation button above a real one, so
//! they are refused. Scrolling is refused too: a document sits in the
//! application's own scrolling, and a scrolling box inside a button is a
//! tree the native host rejects.

/// The most classes one element may carry.
const MAX_CLASSES: usize = 32;
/// The longest class attribute, in bytes.
const MAX_LENGTH: usize = 512;

/// Utilities refused at the start of a class, after any variant prefix such
/// as `md:` or `dark:`.
const REFUSED_PREFIXES: &[&str] = &[
	"absolute",
	"fixed",
	"sticky",
	"inset",
	"top-",
	"right-",
	"bottom-",
	"left-",
	"start-",
	"end-",
	"z-",
	"translate",
	"-",
	"w-screen",
	"h-screen",
	"min-h-screen",
	"min-w-screen",
	"overflow-visible",
	"overflow-scroll",
	"overflow-auto",
	"overflow-x-",
	"overflow-y-",
];

/// Checks a document's `class` attribute, returning why it is refused.
pub(crate) fn check(classes: &str) -> Result<(), String> {
	if classes.len() > MAX_LENGTH {
		return Err(format!(
			"a class attribute is limited to {MAX_LENGTH} bytes"
		));
	}

	let mut count = 0;
	for class in classes.split_whitespace() {
		count += 1;
		if count > MAX_CLASSES {
			return Err(format!("an element is limited to {MAX_CLASSES} classes"));
		}

		let utility = class.rsplit(':').next().unwrap_or(class);
		let utility = utility.strip_prefix('!').unwrap_or(utility);
		if let Some(prefix) = REFUSED_PREFIXES
			.iter()
			.find(|prefix| utility.starts_with(**prefix))
		{
			let what = match *prefix {
				"-" => "negative offsets",
				prefix if prefix.starts_with("overflow-") && prefix != "overflow-visible" => {
					"scrolling"
				}
				prefix => prefix.trim_end_matches('-'),
			};
			return Err(format!(
				"`{class}` is not allowed: agent content stays in its own box, so {what} \
				 utilities are refused"
			));
		}
	}

	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn allows_layout_inside_the_box() {
		assert!(check("flex-row gap-4 p-4 rounded-xl md:flex-col dark:bg-neutral-800").is_ok());
		assert!(check("w-full max-w-[24px] text-sm font-semibold").is_ok());
	}

	#[test]
	fn refuses_classes_that_escape_the_box() {
		for class in [
			"fixed",
			"absolute inset-0",
			"md:absolute",
			"top-0",
			"z-50",
			"-mt-96",
			"translate-x-4",
			"h-screen",
			"!fixed",
			"overflow-scroll",
			"overflow-y-auto",
		] {
			assert!(check(class).is_err(), "{class} should be refused");
		}
	}

	#[test]
	fn limits_size() {
		assert!(check(&"p-1 ".repeat(40)).is_err());
		assert!(check(&"x".repeat(600)).is_err());
	}
}
