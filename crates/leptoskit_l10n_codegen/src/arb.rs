//! Reading ARB files: one JSON object per locale, holding messages, `@key`
//! metadata, and `@@` globals.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::path::Path;
use std::path::PathBuf;

use serde_json::Value;
use unic_langid::LanguageIdentifier;

use crate::Diagnostic;
use crate::catalog::PlaceholderType;

/// One locale's file, read and checked on its own.
#[derive(Debug)]
pub(crate) struct ArbFile {
	pub locale: LanguageIdentifier,
	/// The canonical tag, such as `pt-BR`.
	pub tag: String,
	pub path: PathBuf,
	/// The file name, such as `pt-BR.arb`.
	pub name: String,
	pub messages: BTreeMap<String, Message>,
}

/// One message as written, with its `@key` metadata.
#[derive(Debug)]
pub(crate) struct Message {
	pub text: String,
	pub line: usize,
	pub description: Option<String>,
	/// The placeholders `@key` declares, with their types where given.
	pub declared: BTreeMap<String, Option<PlaceholderType>>,
}

/// Reads every `.arb` file in `dir`, sorted by name. Files that cannot be
/// read are reported and left out.
pub(crate) fn read_dir(dir: &Path, diagnostics: &mut Vec<Diagnostic>) -> Vec<ArbFile> {
	let mut paths: Vec<PathBuf> = match std::fs::read_dir(dir) {
		Ok(entries) => {
			entries
				.filter_map(Result::ok)
				.map(|entry| entry.path())
				.filter(|path| path.is_file() && path.extension().is_some_and(|ext| ext == "arb"))
				.collect()
		}
		Err(error) => {
			diagnostics.push(Diagnostic::new(
				dir,
				None,
				format!("the locales directory could not be read: {error}"),
			));
			return Vec::new();
		}
	};
	paths.sort();
	let mut files: Vec<ArbFile> = Vec::new();
	for path in paths {
		let name = path
			.file_name()
			.map(|name| name.to_string_lossy().into_owned())
			.unwrap_or_default();
		let stem = path
			.file_stem()
			.map(|stem| stem.to_string_lossy().into_owned())
			.unwrap_or_default();
		let Ok(locale) = stem.parse::<LanguageIdentifier>() else {
			diagnostics.push(Diagnostic::new(
				&path,
				None,
				format!(
					"`{name}` is not named after a locale; name it after its language tag, such \
					 as `en.arb` or `pt-BR.arb`"
				),
			));
			continue;
		};
		if let Some(other) = files.iter().find(|file| file.locale == locale) {
			diagnostics.push(Diagnostic::new(
				&path,
				None,
				format!(
					"`{name}` and `{}` are both the `{}` locale; keep one",
					other.name, other.tag
				),
			));
			continue;
		}
		if let Some(file) = read_file(&path, name, locale, diagnostics) {
			files.push(file);
		}
	}
	files
}

fn read_file(
	path: &Path,
	name: String,
	locale: LanguageIdentifier,
	diagnostics: &mut Vec<Diagnostic>,
) -> Option<ArbFile> {
	let raw = match std::fs::read_to_string(path) {
		Ok(raw) => raw,
		Err(error) => {
			diagnostics.push(Diagnostic::new(
				path,
				None,
				format!("the file could not be read: {error}"),
			));
			return None;
		}
	};
	let value: Value = match serde_json::from_str(&raw) {
		Ok(value) => value,
		Err(error) => {
			let line = (error.line() > 0).then_some(error.line());
			let message = error.to_string();
			let message = message
				.rfind(" at line ")
				.map_or(message.as_str(), |at| &message[..at]);
			diagnostics.push(Diagnostic::new(
				path,
				line,
				format!("the file is not JSON: {message}"),
			));
			return None;
		}
	};
	let Value::Object(object) = value else {
		diagnostics.push(Diagnostic::new(
			path,
			Some(1),
			"the file must hold one JSON object, with a message per key".to_owned(),
		));
		return None;
	};
	let tag = locale.to_string();
	let mut lines = BTreeMap::new();
	for (key, line) in key_lines(&raw) {
		match lines.entry(key) {
			Entry::Occupied(first) => {
				diagnostics.push(Diagnostic::new(
					path,
					Some(line),
					format!("`{}` is written twice; keep one", first.key()),
				));
			}
			Entry::Vacant(vacant) => {
				vacant.insert(line);
			}
		}
	}
	let line_of = |key: &str| lines.get(key).copied();

	let mut messages = BTreeMap::new();
	let mut metadata = Vec::new();
	for (key, value) in object {
		let line = line_of(&key);
		if key == "@@locale" {
			let written = value
				.as_str()
				.and_then(|tag| tag.parse::<LanguageIdentifier>().ok());
			if written.as_ref() != Some(&locale) {
				diagnostics.push(Diagnostic::new(
					path,
					line,
					format!("`@@locale` must be `{tag}`, the locale the file is named after"),
				));
			}
		} else if key.starts_with("@@") {
			// Other globals, such as `@@last_modified`, are the translation
			// tool's business.
		} else if let Some(base) = key.strip_prefix('@') {
			metadata.push((base.to_owned(), key.clone(), value, line));
		} else if let Some(text) = value.as_str() {
			messages.insert(
				key,
				Message {
					text: text.to_owned(),
					line: line.unwrap_or(1),
					description: None,
					declared: BTreeMap::new(),
				},
			);
		} else {
			diagnostics.push(Diagnostic::new(
				path,
				line,
				format!("`{key}` must be a string: the message's text"),
			));
		}
	}
	for (base, key, value, line) in metadata {
		let Some(message) = messages.get_mut(&base) else {
			diagnostics.push(Diagnostic::new(
				path,
				line,
				format!("`{key}` describes `{base}`, which is not a message in this file"),
			));
			continue;
		};
		read_metadata(path, &key, &value, line, message, diagnostics);
	}
	Some(ArbFile {
		locale,
		tag,
		path: path.to_path_buf(),
		name,
		messages,
	})
}

/// Reads an `@key` object: its `description` and its `placeholders`.
fn read_metadata(
	path: &Path,
	key: &str,
	value: &Value,
	line: Option<usize>,
	message: &mut Message,
	diagnostics: &mut Vec<Diagnostic>,
) {
	let Value::Object(object) = value else {
		diagnostics.push(Diagnostic::new(
			path,
			line,
			format!("`{key}` must be an object, with a `description` and `placeholders`"),
		));
		return;
	};
	match object.get("description") {
		None => {}
		Some(Value::String(description)) => message.description = Some(description.clone()),
		Some(_) => {
			diagnostics.push(Diagnostic::new(
				path,
				line,
				format!("the `description` of `{key}` must be a string"),
			));
		}
	}
	let placeholders = match object.get("placeholders") {
		None => return,
		Some(Value::Object(placeholders)) => placeholders,
		Some(_) => {
			diagnostics.push(Diagnostic::new(
				path,
				line,
				format!(
					"the `placeholders` of `{key}` must be an object, with a key per placeholder"
				),
			));
			return;
		}
	};
	for (name, spec) in placeholders {
		let Value::Object(spec) = spec else {
			diagnostics.push(Diagnostic::new(
				path,
				line,
				format!("the placeholder `{name}` of `{key}` must be an object, such as `{{}}`"),
			));
			continue;
		};
		let kind = match spec.get("type") {
			None => None,
			Some(Value::String(kind)) => {
				let Some(known) = PlaceholderType::from_arb(kind) else {
					diagnostics.push(Diagnostic::new(
						path,
						line,
						format!(
							"the placeholder `{name}` of `{key}` has type `{kind}`, which is not \
							 supported: use String, int, num, or double"
						),
					));
					continue;
				};
				Some(known)
			}
			Some(_) => {
				diagnostics.push(Diagnostic::new(
					path,
					line,
					format!("the `type` of placeholder `{name}` of `{key}` must be a string"),
				));
				continue;
			}
		};
		message.declared.insert(name.clone(), kind);
	}
}

/// The line of every top-level key, in file order, duplicates included.
fn key_lines(raw: &str) -> Vec<(String, usize)> {
	let bytes = raw.as_bytes();
	let mut keys = Vec::new();
	let mut depth = 0_usize;
	let mut line = 1;
	let mut index = 0;
	while index < bytes.len() {
		match bytes[index] {
			b'\n' => {
				line += 1;
				index += 1;
			}
			b'{' | b'[' => {
				depth += 1;
				index += 1;
			}
			b'}' | b']' => {
				depth = depth.saturating_sub(1);
				index += 1;
			}
			b'"' => {
				let start = index;
				let start_line = line;
				index += 1;
				while index < bytes.len() && bytes[index] != b'"' {
					if bytes[index] == b'\\' {
						index += 1;
					}
					if bytes.get(index) == Some(&b'\n') {
						line += 1;
					}
					index += 1;
				}
				let end = index;
				index += 1;
				if depth == 1 {
					let mut after = index;
					while bytes.get(after).is_some_and(u8::is_ascii_whitespace) {
						after += 1;
					}
					if bytes.get(after) == Some(&b':') {
						if let Some(key) = raw
							.get(start..=end)
							.and_then(|quoted| serde_json::from_str::<String>(quoted).ok())
						{
							keys.push((key, start_line));
						}
					}
				}
			}
			_ => index += 1,
		}
	}
	keys
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn key_lines_find_top_level_keys_only() {
		let raw = "{\n\t\"a\": \"x: {y}\",\n\t\"@a\": {\n\t\t\"description\": \
		           \"d\",\n\t\t\"placeholders\": { \"name\": {} }\n\t},\n\t\"esc\\\"aped\": [ \
		           \"not: a key\" ],\n\t\"a\": \"again\"\n}";
		assert_eq!(
			key_lines(raw),
			[
				("a".to_owned(), 2),
				("@a".to_owned(), 3),
				("esc\"aped".to_owned(), 7),
				("a".to_owned(), 8),
			]
		);
		assert_eq!(key_lines(""), []);
		assert_eq!(key_lines("{\"unterminated"), []);
		assert_eq!(key_lines("{\"k\"\n:1}"), [("k".to_owned(), 1)]);
	}
}
