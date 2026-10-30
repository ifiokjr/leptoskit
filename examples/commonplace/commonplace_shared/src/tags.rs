//! Tags: short lowercase words, stored comma-separated on each clip.

/// The tags in a typed string, normalized: trimmed, lowercased, spaces as
/// dashes, without duplicates or empties, in the order first written.
#[must_use]
pub fn parse(input: &str) -> Vec<String> {
	let mut tags: Vec<String> = Vec::new();
	for tag in input.split([',', '#']) {
		let tag = tag
			.trim()
			.to_lowercase()
			.split_whitespace()
			.collect::<Vec<_>>()
			.join("-");
		if !tag.is_empty() && !tags.contains(&tag) {
			tags.push(tag);
		}
	}
	tags
}

/// Tags as stored: `parse`d, then joined with commas.
#[must_use]
pub fn normalize(input: &str) -> String {
	parse(input).join(",")
}

#[cfg(test)]
mod tests {
	#[test]
	fn tags_are_normalized() {
		assert_eq!(
			super::normalize(" Method, #Urban Heat ,method,, "),
			"method,urban-heat"
		);
	}
}
