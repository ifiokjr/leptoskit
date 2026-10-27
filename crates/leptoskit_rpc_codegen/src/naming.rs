//! Identifier conversion for the target languages.
//!
//! Wire names come from `serde` and are not negotiable; what each generator
//! controls is the identifier it binds them to. The rules here keep generated
//! code valid (no keywords, no illegal characters) and idiomatic for the
//! language (`camelCase` methods in TypeScript, `snake_case` fields in Rust
//! and Python, `PascalCase` types everywhere).

use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt::Write as _;

use heck::ToLowerCamelCase;
use heck::ToSnakeCase;
use heck::ToUpperCamelCase;

use crate::spec::ApiSpec;

/// The identifiers already bound in one namespace of a generated client: a
/// module's types, a class's members, a function's parameters.
///
/// Schema and function names come from the app and can be anything, while
/// every client's runtime claims names of its own (`Client`, `FnError`, the
/// transport's methods) and leans on the language's built-ins. Binding every
/// generated name through a scope seeded with those keeps a model or method
/// from shadowing them, and two generated names from landing on one
/// identifier; a clash gets the first free numeric suffix.
#[derive(Debug, Clone)]
pub(crate) struct Scope {
	reserved: &'static [&'static str],
	taken: HashSet<String>,
}

impl Scope {
	/// An empty scope in which `reserved` is never bound.
	pub(crate) fn new(reserved: &'static [&'static str]) -> Self {
		Self {
			reserved,
			taken: HashSet::new(),
		}
	}

	/// Binds `wanted`, or `wanted2`, `wanted3`, … when it is reserved or
	/// already bound.
	pub(crate) fn bind(&mut self, wanted: &str) -> String {
		let mut name = wanted.to_owned();
		let mut counter = 2;
		while self.reserved.contains(&name.as_str()) || !self.taken.insert(name.clone()) {
			name = format!("{wanted}{counter}");
			counter += 1;
		}
		name
	}
}

/// Binds every component schema's type name in `scope`, in spec order,
/// keyed by schema name: declarations and references then agree on a name
/// even when it had to change, and nothing synthesized later can take it.
pub(crate) fn component_names(spec: &ApiSpec, scope: &mut Scope) -> HashMap<String, String> {
	spec.schemas
		.keys()
		.map(|name| (name.clone(), scope.bind(&type_name(name))))
		.collect()
}

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

/// Type names the TypeScript module declares or calls, which no component
/// schema may take: the runtime's own types, and the globals its code uses
/// (a model named `Record` would redefine `Record<string, string>`).
pub(crate) const TS_RESERVED_TYPES: &[&str] = &[
	"Array",
	"BaseClient",
	"Client",
	"ClientOptions",
	"Encoding",
	"Error",
	"FnError",
	"FnErrorKind",
	"JSON",
	"Object",
	"Promise",
	"Record",
	"Response",
	"String",
	"URLSearchParams",
];

/// The members of `BaseClient` and `Object.prototype`, which no generated
/// method may override: a method named `toString` would send a request
/// whenever the client is printed.
pub(crate) const TS_RESERVED_METHODS: &[&str] = &[
	"call",
	"constructor",
	"fetchImpl",
	"hasOwnProperty",
	"headers",
	"isPrototypeOf",
	"propertyIsEnumerable",
	"serverUrl",
	"toLocaleString",
	"toString",
	"valueOf",
];

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

/// Names the Python modules define or import next to the models: the
/// runtime's classes, the `typing` names, and the built-ins the transport
/// raises or tests (`client.py` imports every model by name).
pub(crate) const PY_RESERVED_TYPES: &[&str] = &[
	"Any",
	"AsyncClient",
	"Client",
	"Exception",
	"False",
	"FnError",
	"ImportError",
	"Literal",
	"Mapping",
	"None",
	"NotRequired",
	"Optional",
	"Required",
	"True",
	"TypeAlias",
	"TypedDict",
	"Union",
	"ValueError",
];

/// The attributes and methods the clients define, which a generated method
/// would shadow or be shadowed by.
pub(crate) const PY_RESERVED_METHODS: &[&str] =
	&["aclose", "base_url", "headers", "http", "timeout"];

/// Parameter names a method signature already uses.
pub(crate) const PY_RESERVED_PARAMETERS: &[&str] = &["self"];

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

/// Package-level names the Go runtime declares, which models and enum
/// constants share a namespace with.
pub(crate) const GO_RESERVED: &[&str] = &[
	"Client",
	"DefaultServerURL",
	"FnError",
	"FnErrorKind",
	"KindDecode",
	"KindInvalid",
	"KindNetwork",
	"KindOther",
	"KindServer",
	"NewClient",
];

/// The fields of `Client`, which a method may not share a name with.
pub(crate) const GO_RESERVED_METHODS: &[&str] = &["HTTPClient", "ServerURL"];

/// Names a generated method's body already uses: the receiver, the context,
/// the result, and the predeclared identifiers its types and returns spell.
pub(crate) const GO_RESERVED_PARAMETERS: &[&str] = &[
	"any", "bool", "c", "ctx", "error", "float64", "int8", "int16", "int32", "int64", "nil",
	"result", "string", "uint8", "uint16", "uint32", "uint64",
];

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

/// Type names `client.rs` declares or imports next to its glob import of the
/// models, plus the prelude types every generated file spells: a model named
/// `Error` would otherwise lose to the client's own, and one named `Option`
/// would redefine every optional field.
pub(crate) const RUST_RESERVED_TYPES: &[&str] = &[
	"Box",
	"Client",
	"Default",
	"DeserializeOwned",
	"Encoding",
	"Error",
	"FnErrorKind",
	"Option",
	"Result",
	"Serialize",
	"String",
	"Vec",
];

/// [`RUST_RESERVED_TYPES`] plus what the CLI's `main.rs` declares or
/// imports next to its glob import of the client.
pub(crate) const RUST_CLI_RESERVED_TYPES: &[&str] = &[
	"Box",
	"Cli",
	"Client",
	"Command",
	"Default",
	"DeserializeOwned",
	"Encoding",
	"Error",
	"FnErrorKind",
	"Json",
	"Option",
	"Parser",
	"Result",
	"Serialize",
	"String",
	"Subcommand",
	"Vec",
];

/// The methods `Client` has, inherent or derived, which a generated method
/// would duplicate or shadow.
pub(crate) const RUST_RESERVED_METHODS: &[&str] = &[
	"call",
	"call_void",
	"clone",
	"default",
	"exchange",
	"finish",
	"fmt",
	"new",
	"server_url",
	"with_header",
	"with_server_url",
];

/// Flags clap or the CLI already defines, which an argument would collide
/// with.
pub(crate) const RUST_CLI_RESERVED_ARGUMENTS: &[&str] = &["help", "server_url", "version"];

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

/// Library-level names the Dart runtime declares, plus the `dart:core` and
/// `dart:typed_data` types its code spells.
pub(crate) const DART_RESERVED_TYPES: &[&str] = &[
	"Client",
	"Exception",
	"FnError",
	"Function",
	"Future",
	"List",
	"Map",
	"MapEntry",
	"Object",
	"String",
	"Uint8List",
	"Uri",
	"defaultServerUrl",
	"toQueryString",
];

/// Members `Client` declares or inherits from `Object`, which a generated
/// method may not take.
pub(crate) const DART_RESERVED_METHODS: &[&str] = &[
	"hashCode",
	"noSuchMethod",
	"runtimeType",
	"serverUrl",
	"toString",
];

/// Members every model class declares or inherits, which a field may not
/// take.
pub(crate) const DART_RESERVED_FIELDS: &[&str] = &[
	"fromJson",
	"hashCode",
	"noSuchMethod",
	"runtimeType",
	"toJson",
	"toString",
];

/// Locals a generated method's body declares.
pub(crate) const DART_RESERVED_PARAMETERS: &[&str] = &["result"];

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
	fn scopes_keep_reserved_and_bound_names_apart() {
		let mut scope = Scope::new(&["Client", "Client2"]);
		assert_eq!(
			scope.bind("Client"),
			"Client3",
			"reserved names are skipped"
		);
		assert_eq!(scope.bind("Todo"), "Todo");
		assert_eq!(scope.bind("Todo"), "Todo2", "bound names are not reused");
		assert_eq!(scope.bind("Todo2"), "Todo22");

		let mut spec = ApiSpec::empty("demo", "0.1.0");
		for name in ["Client", "todo_item", "TodoItem"] {
			spec.schemas
				.insert(name.to_owned(), serde_json::json!({ "type": "string" }));
		}
		let names = component_names(&spec, &mut Scope::new(&["Client"]));
		assert_eq!(names["Client"], "Client2");
		assert_eq!(names["todo_item"], "TodoItem");
		assert_eq!(
			names["TodoItem"], "TodoItem2",
			"schema names that case alike still get their own type"
		);
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
