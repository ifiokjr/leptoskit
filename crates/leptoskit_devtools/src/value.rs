//! Values as the `DevTools` show them: JSON with a readable Rust type name.

use leptoskit_devtools_protocol::Value;
use serde::Serialize;

/// `T`'s name without module paths, the way the `DevTools` show it:
/// `alloc::vec::Vec<app::Clip>` reads `Vec<Clip>`.
pub(crate) fn type_name<T: ?Sized>() -> String {
	short_type_name(std::any::type_name::<T>())
}

/// A type name from [`std::any::type_name`] without its module paths.
pub(crate) fn short_type_name(full: &str) -> String {
	let mut short = String::with_capacity(full.len());
	// Where the path being read started in `short`.
	let mut path = 0;
	let mut rest = full;

	while let Some(character) = rest.chars().next() {
		// A `::` after a name ends a module: drop the module. One after
		// anything else, as in `<T as Trait>::Output`, stays.
		if let Some(after) = rest.strip_prefix("::")
			&& short.len() > path
		{
			short.truncate(path);
			rest = after;
			continue;
		}

		if !(character.is_alphanumeric() || character == '_') {
			path = short.len() + character.len_utf8();
		}
		short.push(character);
		rest = &rest[character.len_utf8()..];
	}

	short
}

/// `value` as JSON, named after `T`. A value that can't be serialized says
/// why instead, so one bad value never hides the rest.
pub(crate) fn encode<T: Serialize + ?Sized>(value: &T) -> Value {
	let type_name = type_name::<T>();
	match serde_json::to_value(value) {
		Ok(json) => Value::from_json(type_name, json),
		Err(error) => {
			Value::from_json(
				type_name,
				serde_json::json!({ "error": format!("can't be shown: {error}") }),
			)
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn type_names_drop_their_module_paths() {
		assert_eq!(
			short_type_name("alloc::vec::Vec<commonplace_shared::Clip>"),
			"Vec<Clip>"
		);
		assert_eq!(
			short_type_name("core::option::Option<(u32, alloc::string::String)>"),
			"Option<(u32, String)>"
		);
		assert_eq!(
			short_type_name("std::collections::hash::map::HashMap<&str, [u8; 4]>"),
			"HashMap<&str, [u8; 4]>"
		);
		assert_eq!(short_type_name("dyn core::fmt::Debug"), "dyn Debug");
		assert_eq!(
			short_type_name("<app::Clip as app::Model>::Id"),
			"<Clip as Model>::Id"
		);
		assert_eq!(type_name::<Vec<String>>(), "Vec<String>");
		assert_eq!(type_name::<usize>(), "usize");
	}

	#[test]
	fn a_value_that_cannot_be_serialized_says_why() {
		let mut map = std::collections::BTreeMap::new();
		map.insert(vec![1_u8], 2_u8);
		let value = encode(&map);
		assert!(!value.truncated);
		assert!(
			value.json["error"]
				.as_str()
				.is_some_and(|error| error.starts_with("can't be shown"))
		);
	}

	#[test]
	fn large_values_carry_the_truncation_marker() {
		let value = encode(&"x".repeat(leptoskit_devtools_protocol::MAX_VALUE_BYTES + 1));
		assert!(value.truncated);
		assert_eq!(value.type_name, "String");
		assert_eq!(value.json["truncated"], true);
	}
}
