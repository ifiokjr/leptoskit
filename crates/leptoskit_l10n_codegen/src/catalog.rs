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

	/// The largest `=N` a plural over this type can compare exactly: an
	/// `i64` up to its maximum, and an `f64` up to 2^53, the last whole
	/// number before it starts rounding.
	fn largest_exact(self) -> u64 {
		match self {
			Self::Int => i64::MAX.unsigned_abs(),
			Self::Num => 1 << f64::MANTISSA_DIGITS,
			Self::Str => u64::MAX,
		}
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
	/// By method name.
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

/// The methods the `L10n` handle answers to before its messages, which a
/// message cannot be named: the handle's own, and those the prelude's
/// traits give it. The handle dereferences to the messages, and Rust looks
/// at the handle first, so a message named `locale` would be unreachable
/// behind `L10n::locale`.
const HANDLE_METHODS: &[&str] = &[
	// `L10n`'s own.
	"locale",
	"messages",
	"provide",
	"set_locale",
	"supported_locales",
	// `Clone`, and `ToOwned` for every `Clone` type.
	"clone",
	"clone_from",
	"clone_into",
	"to_owned",
	// `Into` and `TryInto`, which every type has.
	"into",
	"try_into",
];

/// Keywords `syn` still reads as identifiers, because they are reserved
/// only from an edition on: `gen` from 2024.
const EDITION_KEYWORDS: &[&str] = &["gen"];

/// Why a name in snake case cannot be a Rust identifier.
enum NotAnIdentifier {
	/// Nothing is left of the name in snake case.
	Empty,
	/// It starts with a digit.
	Digit,
	/// It is a keyword, such as `type` or `gen`.
	Keyword,
	/// It holds a character no identifier can, such as `²`.
	Character(char),
}

impl NotAnIdentifier {
	fn describe(&self, snake: &str) -> String {
		match self {
			Self::Empty => "nothing is left of it in snake case".to_owned(),
			Self::Digit => format!("`{snake}` starts with a digit"),
			Self::Keyword => format!("`{snake}` is a Rust keyword"),
			Self::Character(character) => {
				format!("`{snake}` holds `{character}`, which a Rust identifier cannot")
			}
		}
	}
}

/// `snake` as an identifier in every edition.
fn identifier(snake: &str) -> Result<syn::Ident, NotAnIdentifier> {
	if snake.is_empty() {
		return Err(NotAnIdentifier::Empty);
	}

	if snake.starts_with(|first: char| first.is_ascii_digit()) {
		return Err(NotAnIdentifier::Digit);
	}

	// A character continues an identifier when `_` and it make one.
	if let Some(character) = snake
		.chars()
		.find(|character| syn::parse_str::<syn::Ident>(&format!("_{character}")).is_err())
	{
		return Err(NotAnIdentifier::Character(character));
	}

	if EDITION_KEYWORDS.contains(&snake) {
		return Err(NotAnIdentifier::Keyword);
	}

	syn::parse_str::<syn::Ident>(snake).map_err(|_| NotAnIdentifier::Keyword)
}

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
	// By tag rather than by file name, which can write a tag several ways.
	files.sort_by(|left, right| left.tag.cmp(&right.tag));
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

		if let Some(method) = &method
			&& let Some(previous) = methods.insert(method.to_string(), key)
		{
			diagnostics.push(Diagnostic::new(
				&template.path,
				Some(written.line),
				format!("`{key}` and `{previous}` both become the method `{method}`; rename one"),
			));
			continue;
		}
		let parts = parse_parts(&template.path, key, written, diagnostics);
		let placeholders = parts.as_ref().and_then(|parts| {
			type_placeholders(&template.path, key, written, parts, diagnostics).filter(
				|placeholders| {
					exact_numbers_fit(
						&template.path,
						written.line,
						key,
						parts,
						placeholders,
						diagnostics,
					)
				},
			)
		});

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

/// The method a key becomes: its snake case, when that is a Rust identifier
/// the handle does not answer to itself.
fn method_ident(
	path: &Path,
	key: &str,
	line: usize,
	diagnostics: &mut Vec<Diagnostic>,
) -> Option<syn::Ident> {
	let snake = key.to_snake_case();

	let reason = match identifier(&snake) {
		Ok(ident) if !HANDLE_METHODS.contains(&snake.as_str()) => return Some(ident),
		Ok(_) => {
			format!(
				"`{snake}` is a method of the `L10n` handle itself, which `l10n.{snake}()` would \
				 call instead; rename the key"
			)
		}
		Err(NotAnIdentifier::Empty) => "nothing is left of it; use letters and digits".to_owned(),
		Err(NotAnIdentifier::Digit) => {
			format!("`{snake}` starts with a digit; start the key with a letter")
		}
		Err(not) => format!("{}; rename the key", not.describe(&snake)),
	};

	diagnostics.push(Diagnostic::new(
		path,
		Some(line),
		format!("`{key}` cannot become a method: {reason}"),
	));
	None
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

	// A canonical tag is ASCII letters and digits between hyphens, starting
	// with a letter; in upper camel case that is an identifier, and never the
	// one keyword with a capital, `Self`, since no language subtag is four
	// letters long.
	syn::parse_str(&camel).expect("a tag in upper camel case is an identifier")
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

		let ident = match identifier(&snake) {
			Ok(ident) => ident,
			Err(not) => {
				report(format!(
					"the placeholder `{name}` of `{key}` cannot become a parameter: {}; rename it",
					not.describe(&snake)
				));
				complete = false;
				continue;
			}
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

	complete && exact_numbers_fit(path, line, key, parts, &template.placeholders, diagnostics)
}

/// Whether every `=N` form compares a number its placeholder's type can
/// hold: an `int` holds no more than `i64::MAX`, and a `num` rounds whole
/// numbers past 2^53, so a larger `=N` would not compile or would match its
/// neighbours too.
fn exact_numbers_fit(
	path: &Path,
	line: usize,
	key: &str,
	parts: &[Part],
	placeholders: &[Placeholder],
	diagnostics: &mut Vec<Diagnostic>,
) -> bool {
	let mut fit = true;

	for part in parts {
		let inside: Vec<&[Part]> = match part {
			Part::Text(_) | Part::Placeholder(_) | Part::Number => continue,
			Part::Plural(plural) => {
				if let Some(placeholder) = placeholders
					.iter()
					.find(|placeholder| placeholder.name == plural.arg)
				{
					let largest = placeholder.ty.largest_exact();

					for (number, _) in plural.exact.iter().filter(|(number, _)| *number > largest) {
						diagnostics.push(Diagnostic::new(
							path,
							Some(line),
							format!(
								"`={number}` in `{key}` is out of range for `{}`, {}: its `=N` \
								 forms can be at most {largest}",
								plural.arg,
								match placeholder.ty {
									PlaceholderType::Int => "an int",
									PlaceholderType::Num | PlaceholderType::Str => "a num",
								}
							),
						));
						fit = false;
					}
				}
				plural
					.exact
					.iter()
					.map(|(_, body)| body.as_slice())
					.chain(plural.categories.iter().map(|(_, body)| body.as_slice()))
					.chain([plural.other.as_slice()])
					.collect()
			}
			Part::Select(select) => {
				select
					.cases
					.iter()
					.map(|(_, body)| body.as_slice())
					.chain([select.other.as_slice()])
					.collect()
			}
		};

		for body in inside {
			fit &= exact_numbers_fit(path, line, key, body, placeholders, diagnostics);
		}
	}

	fit
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The methods a trait the handle implements adds to it, for the traits
	/// in the prelude. `Debug` and `Deref` are not in it, so their methods
	/// only shadow a message where the app imports them.
	fn prelude_methods(implemented: &str) -> &'static [&'static str] {
		match implemented {
			"Clone" => &["clone", "clone_from", "clone_into", "to_owned"],
			"Copy" | "Debug" | "Deref" => &[],
			other => {
				panic!(
					"`L10n` implements `{other}`: add the methods it brings into scope to \
					 HANDLE_METHODS, and the trait here"
				)
			}
		}
	}

	#[test]
	fn every_method_the_handle_answers_to_is_reserved() {
		let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../leptoskit_l10n/src/handle.rs");
		let source =
			std::fs::read_to_string(&path).expect("the runtime crate sits beside this one");
		let file = syn::parse_file(&source).expect("handle.rs parses");
		// Every type has `into` and `try_into`, from the prelude's blanket
		// implementations.
		let mut found = vec!["into".to_owned(), "try_into".to_owned()];

		for item in &file.items {
			let syn::Item::Impl(implementation) = item else {
				continue;
			};

			let syn::Type::Path(self_ty) = &*implementation.self_ty else {
				continue;
			};

			if self_ty
				.path
				.segments
				.last()
				.is_none_or(|segment| segment.ident != "L10n")
			{
				continue;
			}
			let Some((implemented, _)) = &implementation.trait_ else {
				for inner in &implementation.items {
					let syn::ImplItem::Fn(function) = inner else {
						continue;
					};

					if function.sig.receiver().is_some()
						&& matches!(function.vis, syn::Visibility::Public(_))
					{
						found.push(function.sig.ident.to_string());
					}
				}

				continue;
			};
			let implemented = implemented
				.segments
				.last()
				.expect("a trait path")
				.ident
				.to_string();
			found.extend(
				prelude_methods(&implemented)
					.iter()
					.map(ToString::to_string),
			);
		}

		assert!(
			found.contains(&"set_locale".to_owned()),
			"read the handle: {found:?}"
		);
		found.sort();
		let mut reserved = HANDLE_METHODS.to_vec();
		reserved.sort_unstable();
		assert_eq!(found, reserved);
	}

	#[test]
	fn identifiers_say_why_a_name_is_not_one() {
		let reason = |snake: &str| {
			identifier(snake).map_or_else(|not| not.describe(snake), |_| "ok".to_owned())
		};
		assert_eq!(reason("app_title"), "ok");
		assert_eq!(reason("größe"), "ok");
		assert_eq!(reason(""), "nothing is left of it in snake case");
		assert_eq!(reason("9lives"), "`9lives` starts with a digit");
		assert_eq!(reason("type"), "`type` is a Rust keyword");
		assert_eq!(reason("self"), "`self` is a Rust keyword");
		// Reserved from edition 2024, though `syn` takes it as a name.
		assert_eq!(reason("gen"), "`gen` is a Rust keyword");

		assert_eq!(
			reason("x²"),
			"`x²` holds `²`, which a Rust identifier cannot"
		);
	}

	#[test]
	fn exact_forms_are_held_to_their_type() {
		assert_eq!(
			PlaceholderType::Int.largest_exact(),
			9_223_372_036_854_775_807
		);
		assert_eq!(PlaceholderType::Num.largest_exact(), 9_007_199_254_740_992);
		// Every whole number up to the limit survives the trip through `f64`.
		let largest = PlaceholderType::Num.largest_exact();
		assert_eq!(largest as f64 as u64, largest);
		assert_ne!((largest + 1) as f64 as u64, largest + 1);
	}
}
