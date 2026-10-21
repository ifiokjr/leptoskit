//! Identifier conversion for the target languages.
//!
//! Wire names come from `serde` and are not negotiable; what each generator
//! controls is the identifier it binds them to. The rules here keep generated
//! code valid (no keywords, no illegal characters) and idiomatic for the
//! language (`camelCase` methods in TypeScript, `snake_case` fields in Rust
//! and Python, `PascalCase` types everywhere).

use std::fmt::Write as _;

use heck::ToLowerCamelCase;
use heck::ToSnakeCase;
use heck::ToUpperCamelCase;

/// Keeps ASCII alphanumerics and underscores, replacing everything else with
/// `_`, and prefixes an underscore when the result would start with a digit.
pub(crate) fn sanitize_identifier(name: &str) -> String {
	let mut out: String = name
		.chars()
		.map(|character| {
			if character.is_ascii_alphanumeric() || character == '_' {
				character
			} else {
				'_'
			}
		})
		.collect();
	if out.is_empty() {
		out.push('_');
	}
	if out.starts_with(|character: char| character.is_ascii_digit()) {
		out.insert(0, '_');
	}
	out
}

/// A schema name as a type name: `UpperCamelCase`, sanitized.
pub(crate) fn type_name(schema_name: &str) -> String {
	let sanitized = sanitize_identifier(schema_name);
	let camel = sanitized.to_upper_camel_case();
	if camel.is_empty() {
		"Unnamed".to_owned()
	} else {
		camel
	}
}

/// A name as kebab-case, for CLI commands.
pub(crate) fn kebab(name: &str) -> String {
	let snake = sanitize_identifier(name).to_snake_case();
	let kebab = snake.replace('_', "-");
	if kebab.is_empty() {
		"command".to_owned()
	} else {
		kebab
	}
}

// --- TypeScript ---------------------------------------------------------

/// A function name as a TypeScript method: `lowerCamelCase`.
pub(crate) fn ts_method(name: &str) -> String {
	let camel = sanitize_identifier(name).to_lower_camel_case();
	if camel.is_empty() {
		"call".to_owned()
	} else {
		camel
	}
}

/// A property key in a TypeScript type literal, quoted when needed.
pub(crate) fn ts_property_key(name: &str) -> String {
	if is_plain_identifier(name) {
		name.to_owned()
	} else {
		ts_string(name)
	}
}

/// A TypeScript string literal with JSON escaping.
pub(crate) fn ts_string(text: &str) -> String {
	serde_json::to_string(text).unwrap_or_else(|_| format!("\"{text}\""))
}

// --- Python -------------------------------------------------------------

const PYTHON_KEYWORDS: &[&str] = &[
	"False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue",
	"def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import",
	"in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while",
	"with", "yield", "match", "case", "type",
];

/// A function or argument name as a Python identifier: `snake_case`, with a
/// trailing underscore when it collides with a keyword.
pub(crate) fn py_identifier(name: &str) -> String {
	let snake = sanitize_identifier(name).to_snake_case();
	let snake = if snake.is_empty() {
		"value".to_owned()
	} else {
		snake
	};
	if PYTHON_KEYWORDS.contains(&snake.as_str()) {
		format!("{snake}_")
	} else {
		snake
	}
}

/// A Python package name: lowercase letters, digits, and underscores.
pub(crate) fn py_package_name(name: &str) -> String {
	let snake = sanitize_identifier(name).to_snake_case();
	if snake.is_empty() {
		"rpc".to_owned()
	} else {
		snake
	}
}

/// A Python string literal with JSON escaping, which Python accepts.
pub(crate) fn py_string(text: &str) -> String {
	serde_json::to_string(text).unwrap_or_else(|_| format!("\"{text}\""))
}

// --- Go -----------------------------------------------------------------

const GO_KEYWORDS: &[&str] = &[
	"break",
	"case",
	"chan",
	"const",
	"continue",
	"default",
	"defer",
	"else",
	"fallthrough",
	"for",
	"func",
	"go",
	"goto",
	"if",
	"import",
	"interface",
	"map",
	"package",
	"range",
	"return",
	"select",
	"struct",
	"switch",
	"type",
	"var",
];

/// A wire name as a Go identifier: `camelCase` (unexported fields), with an
/// underscore suffix for keywords and a prefix for anything starting with a
/// digit.
pub(crate) fn go_identifier(name: &str) -> String {
	let camel = sanitize_identifier(name).to_lower_camel_case();
	let camel = if camel.is_empty() {
		"value".to_owned()
	} else {
		camel
	};
	if GO_KEYWORDS.contains(&camel.as_str()) {
		format!("{camel}_")
	} else {
		camel
	}
}

/// A Go package name: the last segment of the module path, lowercase letters
/// and digits only.
pub(crate) fn go_package_name(name: &str) -> String {
	let last = name.rsplit('/').next().unwrap_or(name);
	let lower: String = sanitize_identifier(last)
		.to_lowercase()
		.chars()
		.filter(|character| character.is_ascii_lowercase() || character.is_ascii_digit())
		.collect();
	if lower.is_empty() {
		"rpc".to_owned()
	} else {
		lower
	}
}

/// A Go string literal with JSON escaping.
pub(crate) fn go_string(text: &str) -> String {
	serde_json::to_string(text).unwrap_or_else(|_| format!("\"{text}\""))
}

// --- Rust ----------------------------------------------------------------

const RUST_KEYWORDS: &[&str] = &[
	"abstract", "as", "async", "await", "become", "box", "break", "const", "continue", "do", "dyn",
	"else", "enum", "extern", "false", "final", "fn", "for", "gen", "if", "impl", "in", "let",
	"loop", "macro", "match", "mod", "move", "mut", "override", "priv", "pub", "ref", "return",
	"static", "struct", "trait", "true", "try", "type", "typeof", "unsafe", "unsized", "use",
	"virtual", "where", "while", "yield",
];

/// A wire name as a Rust field or method: `snake_case`, raw-prefixed for
/// keywords (and suffixed for the few that cannot be raw).
pub(crate) fn rust_identifier(name: &str) -> String {
	let snake = sanitize_identifier(name).to_snake_case();
	let snake = if snake.is_empty() {
		"value".to_owned()
	} else {
		snake
	};
	match snake.as_str() {
		"self" | "Self" | "crate" | "super" => format!("{snake}_"),
		keyword if RUST_KEYWORDS.contains(&keyword) => format!("r#{snake}"),
		_ => snake,
	}
}

/// A Rust string literal.
pub(crate) fn rust_string(text: &str) -> String {
	serde_json::to_string(text).unwrap_or_else(|_| format!("\"{text}\""))
}

// --- Dart ----------------------------------------------------------------

const DART_KEYWORDS: &[&str] = &[
	"abstract",
	"as",
	"assert",
	"async",
	"await",
	"base",
	"break",
	"case",
	"catch",
	"class",
	"const",
	"continue",
	"covariant",
	"default",
	"deferred",
	"do",
	"dynamic",
	"else",
	"enum",
	"export",
	"extends",
	"extension",
	"external",
	"factory",
	"false",
	"final",
	"finally",
	"for",
	"Function",
	"get",
	"hide",
	"if",
	"implements",
	"import",
	"in",
	"interface",
	"is",
	"late",
	"library",
	"mixin",
	"new",
	"null",
	"on",
	"operator",
	"part",
	"required",
	"rethrow",
	"return",
	"sealed",
	"set",
	"show",
	"static",
	"super",
	"switch",
	"sync",
	"this",
	"throw",
	"true",
	"try",
	"typedef",
	"var",
	"void",
	"when",
	"while",
	"with",
	"yield",
];

/// A wire name as a Dart field, parameter, or method: `lowerCamelCase`, with
/// a trailing underscore when it collides with a keyword.
pub(crate) fn dart_identifier(name: &str) -> String {
	let camel = sanitize_identifier(name).to_lower_camel_case();
	let camel = if camel.is_empty() {
		"value".to_owned()
	} else {
		camel
	};
	if DART_KEYWORDS.contains(&camel.as_str()) {
		format!("{camel}_")
	} else {
		camel
	}
}

/// A Dart package name: lowercase letters, digits, and underscores.
pub(crate) fn dart_package_name(name: &str) -> String {
	let snake = sanitize_identifier(name).to_snake_case();
	if snake.is_empty() {
		"rpc".to_owned()
	} else {
		snake
	}
}

/// A Dart string literal: single-quoted, escaping `\`, `'`, `$`, and control
/// characters.
pub(crate) fn dart_string(text: &str) -> String {
	let mut out = String::with_capacity(text.len() + 2);
	out.push('\'');
	for character in text.chars() {
		match character {
			'\\' => out.push_str("\\\\"),
			'\'' => out.push_str("\\'"),
			'$' => out.push_str("\\$"),
			'\n' => out.push_str("\\n"),
			'\r' => out.push_str("\\r"),
			'\t' => out.push_str("\\t"),
			other if other.is_control() => {
				let _ = write!(out, "\\u{{{:x}}}", other as u32);
			}
			other => out.push(other),
		}
	}
	out.push('\'');
	out
}

/// Whether `name` is a valid identifier in every target: ASCII letters,
/// digits, and underscores, not starting with a digit.
pub(crate) fn is_plain_identifier(name: &str) -> bool {
	let mut characters = name.chars();
	characters
		.next()
		.is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
		&& characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn sanitizes_and_cases_names() {
		assert_eq!(type_name("Todo"), "Todo");
		assert_eq!(type_name("Map_of_Todo"), "MapOfTodo");
		assert_eq!(kebab("list_todos"), "list-todos");
		assert_eq!(ts_method("list_todos"), "listTodos");
		assert_eq!(ts_property_key("done"), "done");
		assert_eq!(ts_property_key("created-at"), "\"created-at\"");
		assert_eq!(py_identifier("class"), "class_");
		assert_eq!(py_identifier("listTodos"), "list_todos");
		assert_eq!(py_package_name("@acme/todos-api"), "acme_todos_api");
		assert_eq!(go_identifier("chan"), "chan_");
		assert_eq!(go_identifier("created-at"), "createdAt");
		assert_eq!(go_package_name("example.com/acme/todos-api"), "todosapi");
		assert_eq!(go_package_name("todos"), "todos");
		assert_eq!(rust_identifier("type"), "r#type");
		assert_eq!(rust_identifier("self"), "self_");
		assert_eq!(rust_identifier("created-at"), "created_at");
		assert_eq!(dart_identifier("created_at"), "createdAt");
		assert_eq!(dart_identifier("class"), "class_");
		assert_eq!(dart_package_name("todos-api"), "todos_api");
		assert_eq!(dart_string("it's $5\n"), "'it\\'s \\$5\\n'");
		assert!(!is_plain_identifier("1abc"));
	}

	#[test]
	fn degenerate_names_stay_valid() {
		assert_eq!(sanitize_identifier(""), "_");
		assert_eq!(sanitize_identifier("9lives"), "_9lives");
		assert_eq!(type_name("---"), "Unnamed");
		assert_eq!(
			rust_identifier("_"),
			"value",
			"heck strips lone underscores"
		);
		assert_eq!(rust_identifier("--"), "value");
		assert_eq!(dart_identifier("--"), "value");
		assert_eq!(py_identifier("--"), "value");
		assert_eq!(go_identifier("--"), "value");
		assert_eq!(kebab("--"), "command");
		assert_eq!(dart_package_name("--"), "rpc");
		assert_eq!(go_package_name("--"), "rpc");
		assert_eq!(dart_string("tab\there\r\u{1}"), "'tab\\there\\r\\u{1}'");
		assert_eq!(ts_string("quote\"d"), "\"quote\\\"d\"");
	}
}
