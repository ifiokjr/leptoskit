//! Build-time syntax highlighting, so pages need no highlighting script.
//!
//! Highlighting runs tree-sitter grammars through arborium, so tokens are
//! classed by what they are rather than by regular expressions over the text.
//! Each token becomes a `<span>` with an `hl-` class (`hl-keyword`,
//! `hl-string`, `hl-comment`), and the site's stylesheet colours them.
//!
//! The grammars are the languages documentation for a leptoskit app is
//! written in: Rust and its manifests, shells, the web, the Android and iOS
//! host languages, Nix, PowerShell, Markdown, and diffs.

use std::cell::RefCell;

use arborium::Config;
use arborium::Highlighter;
use arborium::HtmlFormat;

thread_local! {
	static HIGHLIGHTER: RefCell<Highlighter> = RefCell::new(Highlighter::with_config(Config {
		html_format: HtmlFormat::ClassNamesWithPrefix("hl".to_owned()),
		..Config::default()
	}));
}

/// The language of a fence's info string: `rust` for `rust,ignore`, and
/// `None` for plain text.
pub(crate) fn language(info: &str) -> Option<&str> {
	let language = info.split([',', ' ']).next().unwrap_or_default().trim();
	(!language.is_empty() && !matches!(language, "text" | "txt" | "plain")).then_some(language)
}

/// `code` as highlighted HTML, or `None` when no grammar knows `language`.
pub(crate) fn highlight(code: &str, language: &str) -> Result<Option<String>, String> {
	let grammar = match language {
		"console" | "zsh" => "bash",
		other => other,
	};

	HIGHLIGHTER.with_borrow_mut(|highlighter| {
		match highlighter.highlight(grammar, code) {
			Ok(html) => Ok(Some(html)),
			Err(arborium::Error::UnsupportedLanguage { .. }) => Ok(None),
			Err(error) => {
				Err(format!(
					"could not highlight this {language} block: {error}"
				))
			}
		}
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn reads_the_language_from_an_info_string() {
		assert_eq!(language("rust"), Some("rust"));
		assert_eq!(language("rust,ignore"), Some("rust"));
		assert_eq!(language("toml title=\"leptoskit.toml\""), Some("toml"));
		assert_eq!(language("text"), None);
		assert_eq!(language(""), None);
	}

	#[test]
	fn classes_tokens_by_what_they_are() {
		let html = highlight("pub fn main() {}\n", "rust").unwrap().unwrap();

		assert!(
			html.contains("<span class=\"hl-keyword\">fn</span>"),
			"{html}"
		);
		assert!(
			html.contains("<span class=\"hl-function\">main</span>"),
			"{html}"
		);
	}

	#[test]
	fn knows_the_languages_the_docs_use() {
		for language in [
			"rust",
			"rs",
			"toml",
			"bash",
			"sh",
			"shell",
			"console",
			"zsh",
			"json",
			"yaml",
			"nix",
			"kotlin",
			"swift",
			"html",
			"css",
			"javascript",
			"typescript",
			"diff",
			"markdown",
			"powershell",
		] {
			assert!(highlight("x\n", language).unwrap().is_some(), "{language}");
		}
	}

	#[test]
	fn leaves_unknown_languages_plain() {
		assert_eq!(highlight("x", "klingon").unwrap(), None);
	}

	#[test]
	fn escapes_markup_in_code() {
		let html = highlight("let a = \"<b>\";\n", "rust").unwrap().unwrap();

		assert!(html.contains("&lt;b&gt;"), "{html}");
	}
}
