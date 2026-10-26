//! Checking the files against each other and typing every message: the
//! template defines the messages and their placeholders, and every other
//! file translates them.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;

use heck::ToSnakeCase;
use heck::ToUpperCamelCase;
use proc_macro2::TokenStream;
use quote::quote;
use unic_langid::LanguageIdentifier;

use crate::Diagnostic;
use crate::arb;
use crate::arb::ArbFile;
use crate::message;
use crate::message::Part;

/// What a placeholder holds, and the Rust type the method takes it as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlaceholderType {
	/// `String` in ARB: `&str`.
	Str,
	/// `int` in ARB: `i64`.
	Int,
	/// `num` or `double` in ARB: `f64`.
	Num,
}

impl PlaceholderType {
	pub(crate) fn from_arb(name: &str) -> Option<Self> {
		match name {
			"String" => Some(Self::Str),
			"int" => Some(Self::Int),
			"num" | "double" => Some(Self::Num),
			_ => None,
		}
	}

	pub(crate) fn rust(self) -> TokenStream {
		match self {
			Self::Str => quote!(&str),
			Self::Int => quote!(i64),
			Self::Num => quote!(f64),
		}
	}

	fn rust_name(self) -> &'static str {
		match self {
			Self::Str => "&str",
			Self::Int => "i64",
			Self::Num => "f64",
		}
	}

	fn arb_name(self) -> &'static str {
		match self {
			Self::Str => "String",
			Self::Int => "int",
			Self::Num => "num",
		}
	}

	pub(crate) fn is_number(self) -> bool {
		matches!(self, Self::Int | Self::Num)
	}
}

/// One parameter of a message's method.
#[derive(Debug)]
pub(crate) struct Placeholder {
	/// The name in the message, such as `userName`.
	pub name: String,
	/// The parameter, such as `user_name`.
	pub ident: syn::Ident,
	pub ty: PlaceholderType,
}

/// One message, as the template defines it.
#[derive(Debug)]
pub(crate) struct Message {
	pub key: String,
	pub method: syn::Ident,
	pub description: Option<String>,
	/// The template's text, as written.
	pub text: String,
	/// The parameters, in the order the template first uses them.
	pub placeholders: Vec<Placeholder>,
	/// The template's text, parsed.
	pub parts: Vec<Part>,
}

impl Message {
	/// Whether the method returns `&'static str` rather than a `String`:
	/// the message has no placeholders, so it is the same text every time.
	pub(crate) fn is_literal(&self) -> bool {
		self.placeholders.is_empty()
	}

	/// The method's signature, such as `greeting(name: &str) -> String`.
	pub(crate) fn signature(&self) -> String {
		let parameters = self
			.placeholders
			.iter()
			.map(|placeholder| format!("{}: {}", placeholder.ident, placeholder.ty.rust_name()))
			.collect::<Vec<_>>()
			.join(", ");
		let returns = if self.is_literal() {
			"&'static str"
		} else {
			"String"
		};
		format!("{}({parameters}) -> {returns}", self.method)
	}
}

/// One locale's translations.
#[derive(Debug)]
pub(crate) struct Locale {
	pub identifier: LanguageIdentifier,
	/// The canonical tag, such as `pt-BR`.
	pub tag: String,
	/// The unit struct, such as `PtBr`.
	pub ident: syn::Ident,
	/// The file name, such as `pt-BR.arb`.
	pub file: String,
	/// The messages the file translates, parsed, by key.
	pub translated: BTreeMap<String, Vec<Part>>,
	/// The keys the file leaves out, which fall back to the template.
	pub untranslated: Vec<String>,
}

/// Every message and every locale, checked.
#[derive(Debug)]
pub(crate) struct Catalog {
	/// By key.
	pub messages: Vec<Message>,
	/// The template first, then by tag.
	pub locales: Vec<Locale>,
}

impl Catalog {
	pub(crate) fn template(&self) -> &Locale {
		&self.locales[0]
	}
}

/// Names the generated module uses itself, which a locale's struct cannot
/// take.
const RESERVED: &[&str] = &["AppTranslations", "L10n", "Messages"];

/// Builds the catalog, or reports why it cannot. A catalog may come back with
/// diagnostics: the caller treats any diagnostic as failure, and the catalog
/// only shows how far the files got.
pub(crate) fn build(
	mut files: Vec<ArbFile>,
	dir: &Path,
	template: &str,
	diagnostics: &mut Vec<Diagnostic>,
) -> Option<Catalog> {
	let Ok(template_locale) = template.parse::<LanguageIdentifier>() else {
		diagnostics.push(Diagnostic::new(
			dir,
			None,
			format!(
				"`{template}` is not a language tag; set `[l10n] template` to the locale of the \
				 file every other file translates, such as \"en\""
			),
		));
		return None;
	};
	let Some(position) = files.iter().position(|file| file.locale == template_locale) else {
		let message = if files.is_empty() {
			format!(
				"there are no `.arb` files; add `{template_locale}.arb` with a message such as \
				 `\"appTitle\": \"My App\"`"
			)
		} else {
			format!(
				"there is no `{template_locale}.arb`, the template every other file translates; \
				 add it, or set `[l10n] template` to the locale of the file that is"
			)
		};
		diagnostics.push(Diagnostic::new(dir, None, message));
		return None;
	};
	let template = files.remove(position);
	let mut messages = template_messages(&template, diagnostics);
	messages.sort_by_cached_key(|message| message.method.to_string());

	let mut locales = Vec::new();
	let template_ident = struct_ident(&template, &[], diagnostics);
	let translated = messages
		.iter()
		.map(|message| (message.key.clone(), message.parts.clone()))
		.collect();
	locales.push(Locale {
		tag: template.tag.clone(),
		ident: template_ident,
		file: template.name.clone(),
		translated,
		untranslated: Vec::new(),
		identifier: template.locale.clone(),
	});

	for file in files {
		let ident = struct_ident(&file, &locales, diagnostics);
		let mut translated = BTreeMap::new();
		for (key, written) in &file.messages {
			let Some(message) = messages.iter().find(|message| message.key == *key) else {
				// A template message that failed its own checks was reported
				// there; only a key the template never had is news.
				if !template.messages.contains_key(key) {
					diagnostics.push(Diagnostic::new(
						&file.path,
						Some(written.line),
						format!(
							"`{key}` is not in the template `{}`; add it there first, or remove \
							 it here",
							template.name
						),
					));
				}
				continue;
			};
			let Some(parts) = parse_parts(&file.path, key, written, diagnostics) else {
				continue;
			};
			if check_translation(&file.path, written.line, &parts, message, diagnostics) {
				translated.insert(key.clone(), parts);
			}
		}
		let untranslated = messages
			.iter()
			.filter(|message| !file.messages.contains_key(&message.key))
			.map(|message| message.key.clone())
			.collect();
		locales.push(Locale {
			identifier: file.locale,
			tag: file.tag,
			ident,
			file: file.name,
			translated,
			untranslated,
		});
	}
	Some(Catalog { messages, locales })
}

/// Types every message in the template.
fn template_messages(template: &ArbFile, diagnostics: &mut Vec<Diagnostic>) -> Vec<Message> {
	if template.messages.is_empty() {
		diagnostics.push(Diagnostic::new(
			&template.path,
			None,
			"the template has no messages; add one, such as `\"appTitle\": \"My App\"`".to_owned(),
		));
	}
	let mut messages = Vec::new();
	let mut methods: BTreeMap<String, &str> = BTreeMap::new();
	for (key, written) in &template.messages {
		let method = method_ident(&template.path, key, written.line, diagnostics);
		if let Some(method) = &method {
			if let Some(previous) = methods.insert(method.to_string(), key) {
				diagnostics.push(Diagnostic::new(
					&template.path,
					Some(written.line),
					format!(
						"`{key}` and `{previous}` both become the method `{method}`; rename one"
					),
				));
				continue;
			}
		}
		let parts = parse_parts(&template.path, key, written, diagnostics);
		let placeholders = parts
			.as_ref()
			.and_then(|parts| type_placeholders(&template.path, key, written, parts, diagnostics));
		if let (Some(method), Some(parts), Some(placeholders)) = (method, parts, placeholders) {
			messages.push(Message {
				key: key.clone(),
				method,
				description: written.description.clone(),
				text: written.text.clone(),
				placeholders,
				parts,
			});
		}
	}
	messages
}

fn parse_parts(
	path: &Path,
	key: &str,
	written: &arb::Message,
	diagnostics: &mut Vec<Diagnostic>,
) -> Option<Vec<Part>> {
	match message::parse(&written.text) {
		Ok(parts) => Some(parts),
		Err(error) => {
			diagnostics.push(Diagnostic::new(
				path,
				Some(written.line),
				format!("in `{key}`, {error}"),
			));
			None
		}
	}
}

/// The method a key becomes: its snake case, when that is a Rust identifier.
fn method_ident(
	path: &Path,
	key: &str,
	line: usize,
	diagnostics: &mut Vec<Diagnostic>,
) -> Option<syn::Ident> {
	let snake = key.to_snake_case();
	let Ok(ident) = syn::parse_str::<syn::Ident>(&snake) else {
		let reason = if snake.is_empty() {
			"nothing is left of it; use letters and digits".to_owned()
		} else if snake.starts_with(|first: char| first.is_ascii_digit()) {
			format!("`{snake}` starts with a digit; start the key with a letter")
		} else {
			format!("`{snake}` is a Rust keyword; rename the key")
		};
		diagnostics.push(Diagnostic::new(
			path,
			Some(line),
			format!("`{key}` cannot become a method: {reason}"),
		));
		return None;
	};
	Some(ident)
}

/// The unit struct a locale becomes: its tag in upper camel case.
fn struct_ident(file: &ArbFile, taken: &[Locale], diagnostics: &mut Vec<Diagnostic>) -> syn::Ident {
	let camel = file.tag.to_upper_camel_case();
	if RESERVED.contains(&camel.as_str()) {
		diagnostics.push(Diagnostic::new(
			&file.path,
			None,
			format!(
				"the `{}` locale would be named `{camel}`, which the generated module needs for \
				 itself; this locale cannot be used",
				file.tag
			),
		));
	} else if let Some(other) = taken.iter().find(|locale| locale.ident == camel) {
		diagnostics.push(Diagnostic::new(
			&file.path,
			None,
			format!(
				"the `{}` and `{}` locales would both be named `{camel}`; keep one",
				file.tag, other.tag
			),
		));
	}
	syn::parse_str(&camel).unwrap_or_else(|_| quote::format_ident!("Locale"))
}

/// How a message uses a placeholder.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Usage {
	/// `{name}`: shown.
	Shown,
	/// `{name, plural, ...}`: counted.
	Counted,
	/// `{name, select, ...}`: matched.
	Matched,
}

impl Usage {
	fn describe(self) -> &'static str {
		match self {
			Self::Shown => "shown",
			Self::Counted => "counted by a plural",
			Self::Matched => "matched by a select",
		}
	}
}

/// Every placeholder a message uses, in order, with how it uses it.
fn usages(parts: &[Part], out: &mut Vec<(String, Usage)>) {
	for part in parts {
		match part {
			Part::Text(_) | Part::Number => {}
			Part::Placeholder(name) => out.push((name.clone(), Usage::Shown)),
			Part::Plural(plural) => {
				out.push((plural.arg.clone(), Usage::Counted));
				for (_, body) in &plural.exact {
					usages(body, out);
				}
				for (_, body) in &plural.categories {
					usages(body, out);
				}
				usages(&plural.other, out);
			}
			Part::Select(select) => {
				out.push((select.arg.clone(), Usage::Matched));
				for (_, body) in &select.cases {
					usages(body, out);
				}
				usages(&select.other, out);
			}
		}
	}
}

/// Types a template message's placeholders from how it uses them and what
/// its `@key` declares.
fn type_placeholders(
	path: &Path,
	key: &str,
	written: &arb::Message,
	parts: &[Part],
	diagnostics: &mut Vec<Diagnostic>,
) -> Option<Vec<Placeholder>> {
	let mut used = Vec::new();
	usages(parts, &mut used);
	let mut names: Vec<&str> = Vec::new();
	for (name, _) in &used {
		if !names.contains(&name.as_str()) {
			names.push(name);
		}
	}
	let mut placeholders: Vec<Placeholder> = Vec::new();
	let mut complete = true;
	let mut report = |message: String| {
		diagnostics.push(Diagnostic::new(path, Some(written.line), message));
	};
	for name in &names {
		let used_as = |usage| used.iter().any(|(used, how)| used == name && *how == usage);
		let counted = used_as(Usage::Counted);
		let matched = used_as(Usage::Matched);
		if counted && matched {
			report(format!(
				"`{name}` is counted by a plural and matched by a select in `{key}`; use two \
				 placeholders"
			));
			complete = false;
			continue;
		}
		let declared = written.declared.get(*name).copied().flatten();
		let ty = declared.unwrap_or(if counted {
			PlaceholderType::Int
		} else {
			PlaceholderType::Str
		});
		if counted && !ty.is_number() {
			report(format!(
				"`{name}` is counted by a plural in `{key}`, so its type must be int or num, not \
				 {}",
				ty.arb_name()
			));
			complete = false;
		}
		if matched && ty != PlaceholderType::Str {
			report(format!(
				"`{name}` is matched by a select in `{key}`, so its type must be String, not {}",
				ty.arb_name()
			));
			complete = false;
		}
		let snake = name.to_snake_case();
		let Ok(ident) = syn::parse_str::<syn::Ident>(&snake) else {
			report(format!(
				"the placeholder `{name}` of `{key}` cannot become a parameter: `{snake}` is not \
				 a Rust identifier; rename it"
			));
			complete = false;
			continue;
		};
		if let Some(other) = placeholders
			.iter()
			.find(|placeholder| placeholder.ident == ident)
		{
			report(format!(
				"the placeholders `{name}` and `{}` of `{key}` would both be the parameter \
				 `{ident}`; rename one",
				other.name
			));
			complete = false;
			continue;
		}
		placeholders.push(Placeholder {
			name: (*name).to_owned(),
			ident,
			ty,
		});
	}
	for declared in written.declared.keys() {
		if !names.contains(&declared.as_str()) {
			report(format!(
				"`@{key}` declares the placeholder `{declared}`, which the message does not use"
			));
			complete = false;
		}
	}
	complete.then_some(placeholders)
}

/// Whether a translation uses the template's placeholders, each in a way
/// its type allows.
fn check_translation(
	path: &Path,
	line: usize,
	parts: &[Part],
	template: &Message,
	diagnostics: &mut Vec<Diagnostic>,
) -> bool {
	let mut used = Vec::new();
	usages(parts, &mut used);
	let names: BTreeSet<&str> = used.iter().map(|(name, _)| name.as_str()).collect();
	let expected: BTreeSet<&str> = template
		.placeholders
		.iter()
		.map(|placeholder| placeholder.name.as_str())
		.collect();
	let key = &template.key;
	let mut complete = true;
	let missing: Vec<String> = expected
		.difference(&names)
		.map(|name| format!("`{{{name}}}`"))
		.collect();
	let extra: Vec<String> = names
		.difference(&expected)
		.map(|name| format!("`{{{name}}}`"))
		.collect();
	if !missing.is_empty() || !extra.is_empty() {
		let mut problems = Vec::new();
		if !missing.is_empty() {
			problems.push(format!("leaves out {}", missing.join(", ")));
		}
		if !extra.is_empty() {
			problems.push(format!(
				"adds {}, which the template does not have",
				extra.join(", ")
			));
		}
		diagnostics.push(Diagnostic::new(
			path,
			Some(line),
			format!(
				"`{key}` {}; a translation uses the template's placeholders",
				problems.join(" and ")
			),
		));
		complete = false;
	}
	for (name, usage) in &used {
		let Some(placeholder) = template
			.placeholders
			.iter()
			.find(|placeholder| placeholder.name == *name)
		else {
			continue;
		};
		let allowed = match usage {
			Usage::Shown => true,
			Usage::Counted => placeholder.ty.is_number(),
			Usage::Matched => placeholder.ty == PlaceholderType::Str,
		};
		if !allowed {
			diagnostics.push(Diagnostic::new(
				path,
				Some(line),
				format!(
					"`{name}` is {} in `{key}`, but the template types it as {}",
					usage.describe(),
					placeholder.ty.arb_name()
				),
			));
			complete = false;
		}
	}
	complete
}
