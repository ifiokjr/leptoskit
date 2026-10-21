//! The file-name grammar: what each name in `src/routes/` means.

/// One URL segment, from a file or directory name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Segment {
	/// `notes`, `sign-in`.
	Static(String),
	/// `[note]`.
	Param(String),
	/// `[[lang]]`.
	OptionalParam(String),
	/// `[...topic]`.
	Rest(String),
	/// `[[...topic]]`.
	OptionalRest(String),
}

impl Segment {
	/// Orders segments from most to least specific when routes compete for a
	/// URL.
	pub(crate) fn rank(&self) -> u8 {
		match self {
			Self::Static(_) => 0,
			Self::Param(_) => 1,
			Self::OptionalParam(_) => 2,
			Self::Rest(_) => 3,
			Self::OptionalRest(_) => 4,
		}
	}

	/// The parameter's name, or `None` for a static segment.
	pub(crate) fn param(&self) -> Option<&str> {
		match self {
			Self::Static(_) => None,
			Self::Param(name)
			| Self::OptionalParam(name)
			| Self::Rest(name)
			| Self::OptionalRest(name) => Some(name),
		}
	}
}

/// What a name in `src/routes/` means.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Name {
	/// A segment of the URL: a route file, or a directory of them.
	Segment(Segment),
	/// `(tabs)/`: shapes the tree, adds nothing to the URL.
	Group(String),
	/// `index.rs`: the directory's own route.
	Index,
	/// `_layout.rs`.
	Layout,
	/// `+not-found.rs`.
	NotFound,
	/// `+html.rs`.
	Html,
	/// `_note_card.rs`: a module beside the routes, never a route.
	Private(String),
}

/// The suffixes the planned platform-overrides extension will read.
const PLATFORMS: [&str; 4] = ["web", "native", "desktop", "mobile"];

/// Parses a file stem (`is_dir` false) or directory name.
pub(crate) fn parse(name: &str, is_dir: bool) -> Result<Name, String> {
	// `[...topic]` is the one place a name holds dots.
	let without_rest = name.replacen("...", "", 1);
	if let Some((_, suffix)) = without_rest.rsplit_once('.') {
		if PLATFORMS.contains(&suffix) {
			return Err(format!(
				"`{name}` is a platform override; per-platform files and folders are an opt-in \
				 extension that leptoskit does not support yet"
			));
		}
		return Err(format!(
			"`{name}` contains `.`, which route names cannot use"
		));
	}
	if let Some(inner) = name
		.strip_prefix("[[")
		.and_then(|rest| rest.strip_suffix("]]"))
	{
		return Ok(Name::Segment(match inner.strip_prefix("...") {
			Some(param) => Segment::OptionalRest(param_name(name, param)?),
			None => Segment::OptionalParam(param_name(name, inner)?),
		}));
	}
	if let Some(inner) = name
		.strip_prefix('[')
		.and_then(|rest| rest.strip_suffix(']'))
	{
		return Ok(Name::Segment(match inner.strip_prefix("...") {
			Some(param) => Segment::Rest(param_name(name, param)?),
			None => Segment::Param(param_name(name, inner)?),
		}));
	}
	if let Some(group) = name
		.strip_prefix('(')
		.and_then(|rest| rest.strip_suffix(')'))
	{
		if !is_dir {
			return Err(format!("`{name}.rs`: a group is a directory, `{name}/`"));
		}
		static_text(name, group)?;
		return Ok(Name::Group(group.to_owned()));
	}
	if let Some(special) = name.strip_prefix('+') {
		return match (special, is_dir) {
			("not-found", false) => Ok(Name::NotFound),
			("html", false) => Ok(Name::Html),
			(_, true) => Err(format!("`{name}/`: names starting with `+` are files")),
			_ => {
				Err(format!(
					"`{name}.rs` is not a route file leptoskit knows; the `+` files are \
					 `+not-found.rs` and `+html.rs`"
				))
			}
		};
	}
	if let Some(private) = name.strip_prefix('_') {
		if is_dir {
			return Err(format!(
				"`{name}/`: private directories are not supported; put shared code in a crate \
				 module, or in a `_file.rs` beside the routes that use it"
			));
		}
		if private == "layout" {
			return Ok(Name::Layout);
		}
		static_text(name, private)?;
		return Ok(Name::Private(name.to_owned()));
	}
	if name == "index" {
		return if is_dir {
			Err("`index/`: `index` names a directory's own route file, `index.rs`".to_owned())
		} else {
			Ok(Name::Index)
		};
	}
	static_text(name, name)?;
	Ok(Name::Segment(Segment::Static(name.to_owned())))
}

fn param_name(name: &str, param: &str) -> Result<String, String> {
	let mut chars = param.chars();
	let valid = chars
		.next()
		.is_some_and(|first| first == '_' || first.is_ascii_alphabetic())
		&& chars.all(|char| char == '_' || char.is_ascii_alphanumeric());
	if valid {
		Ok(param.to_owned())
	} else {
		Err(format!(
			"`{name}`: a parameter name becomes a struct field, so it must be a Rust identifier \
			 like `note_id`"
		))
	}
}

fn static_text(name: &str, text: &str) -> Result<(), String> {
	let valid = !text.is_empty()
		&& text
			.chars()
			.all(|char| char.is_ascii_alphanumeric() || matches!(char, '-' | '_' | '~'));
	if valid {
		Ok(())
	} else {
		Err(format!(
			"`{name}`: route names use ASCII letters, digits, `-`, `_` and `~`, because they are \
			 URL segments"
		))
	}
}

/// The Rust module name for a file or directory name.
pub(crate) fn module_name(name: &Name) -> String {
	match name {
		Name::Segment(Segment::Static(text)) | Name::Group(text) => identifier(text),
		Name::Segment(segment) => identifier(segment.param().expect("not static")),
		Name::Index => "index".to_owned(),
		Name::Layout => "layout".to_owned(),
		Name::NotFound => "not_found".to_owned(),
		Name::Html => "html".to_owned(),
		Name::Private(private) => identifier(private),
	}
}

/// A snake-case identifier for any route name: `-` and `~` become `_`, a
/// leading digit gets a leading `_`, and a keyword gets a trailing `_`.
pub(crate) fn identifier(text: &str) -> String {
	let mut ident: String = text
		.chars()
		.map(|char| {
			if char.is_ascii_alphanumeric() {
				char.to_ascii_lowercase()
			} else {
				'_'
			}
		})
		.collect();
	if ident.starts_with(|char: char| char.is_ascii_digit()) {
		ident.insert(0, '_');
	}
	if syn::parse_str::<syn::Ident>(&ident).is_err() {
		ident.push('_');
	}
	ident
}

/// `sign-in` becomes `SignIn`, `notes` becomes `Notes`.
pub(crate) fn pascal_case(text: &str) -> String {
	text.split(|char: char| !char.is_ascii_alphanumeric())
		.filter(|word| !word.is_empty())
		.map(|word| {
			let mut chars = word.chars();
			let first = chars.next().expect("words are not empty");
			first.to_ascii_uppercase().to_string() + chars.as_str()
		})
		.collect()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn reads_every_kind_of_name() {
		assert_eq!(parse("index", false), Ok(Name::Index));
		assert_eq!(
			parse("sign-in", false),
			Ok(Name::Segment(Segment::Static("sign-in".to_owned())))
		);
		assert_eq!(
			parse("[note]", false),
			Ok(Name::Segment(Segment::Param("note".to_owned())))
		);
		assert_eq!(
			parse("[[lang]]", true),
			Ok(Name::Segment(Segment::OptionalParam("lang".to_owned())))
		);
		assert_eq!(
			parse("[...topic]", false),
			Ok(Name::Segment(Segment::Rest("topic".to_owned())))
		);
		assert_eq!(
			parse("[[...path]]", false),
			Ok(Name::Segment(Segment::OptionalRest("path".to_owned())))
		);
		assert_eq!(parse("(tabs)", true), Ok(Name::Group("tabs".to_owned())));
		assert_eq!(parse("_layout", false), Ok(Name::Layout));
		assert_eq!(parse("+not-found", false), Ok(Name::NotFound));
		assert_eq!(parse("+html", false), Ok(Name::Html));
		assert_eq!(
			parse("_note_card", false),
			Ok(Name::Private("_note_card".to_owned()))
		);
	}

	#[test]
	fn rejects_names_that_cannot_be_routes() {
		for (name, is_dir) in [
			("(tabs)", false),
			("_shared", true),
			("index", true),
			("+layout", false),
			("+not-found", true),
			("[note-id]", false),
			("[]", false),
			("a b", false),
			("notes.json", false),
		] {
			assert!(parse(name, is_dir).is_err(), "{name} should be rejected");
		}
		let error = parse("index.web", false).unwrap_err();
		assert!(error.contains("platform override"), "{error}");
	}

	#[test]
	fn makes_identifiers_and_type_names() {
		assert_eq!(identifier("sign-in"), "sign_in");
		assert_eq!(identifier("404"), "_404");
		assert_eq!(identifier("type"), "type_");
		assert_eq!(identifier("self"), "self_");
		assert_eq!(identifier("Notes"), "notes");
		assert_eq!(pascal_case("sign-in"), "SignIn");
		assert_eq!(pascal_case("notes"), "Notes");
	}
}
