//! Generating the translations module from ARB files on disk.

use std::path::Path;
use std::path::PathBuf;

use leptoskit_l10n_codegen::LocaleEntry;
use leptoskit_l10n_codegen::Status;
use leptoskit_l10n_codegen::WriteError;

/// A locales directory in a temporary project.
struct Locales {
	project: tempfile::TempDir,
}

impl Locales {
	fn new(files: &[(&str, &str)]) -> Self {
		let project = tempfile::tempdir().expect("temp dir");
		std::fs::create_dir_all(project.path().join("locales")).expect("locales dir");
		let locales = Self { project };
		for (name, source) in files {
			locales.write(name, source);
		}
		locales
	}

	fn write(&self, name: &str, source: &str) {
		std::fs::write(self.dir().join(name), source).expect("write locale file");
	}

	fn dir(&self) -> PathBuf {
		self.project.path().join("locales")
	}

	fn output(&self) -> PathBuf {
		self.project.path().join("l10n.rs")
	}

	fn diagnostics(&self, template: &str) -> String {
		let diagnostics = leptoskit_l10n_codegen::generate(&self.dir(), template)
			.expect_err("the files have problems");
		diagnostics
			.0
			.iter()
			.map(|diagnostic| diagnostic.relative_to(self.project.path()))
			.collect::<Vec<_>>()
			.join("\n")
	}
}

const ENGLISH: &str = r#"{
	"@@locale": "en",
	"appTitle": "Field Notes",
	"greeting": "Hello, {name}!",
	"@greeting": {
		"description": "Shown at the top of the home screen.",
		"placeholders": {
			"name": { "type": "String", "example": "Ana" }
		}
	},
	"cartItems": "{count, plural, =0 {Your cart is empty} one {# item in your cart} other {# items in your cart}}",
	"@cartItems": {
		"placeholders": {
			"count": { "type": "int" }
		}
	},
	"distance": "{km, plural, =1 {one kilometre} other {# kilometres}} to go",
	"@distance": {
		"description": "How far the next note is.",
		"placeholders": {
			"km": { "type": "num" }
		}
	},
	"pronoun": "{gender, select, female {she} male {he} other {they}}",
	"inbox": "{who, select, you {You have {count, plural, one {# message} other {# messages}}} other {{who} has {count, plural, one {# message} other {# messages}}}}",
	"quoted": "It''s '{not a placeholder}' with a lone ' and `backticks`",
	"emptyMessage": "",
	"URLLabel": "Open {url}",
	"justName": "{name}",
	"@@last_modified": "2026-10-25T00:00:00Z"
}
"#;

const PORTUGUESE: &str = r#"{
	"@@locale": "pt-BR",
	"appTitle": "Notas de Campo",
	"greeting": "Olá, {name}!",
	"@greeting": {
		"description": "Translators may describe too; the template's types win."
	},
	"cartItems": "{count, plural, =0 {Seu carrinho está vazio} one {# item no carrinho} other {# itens no carrinho}}",
	"pronoun": "{gender, select, male {ele} female {ela} other {elu}}",
	"inbox": "{who, select, you {Você tem {count, plural, one {# mensagem} other {# mensagens}}} other {{who} tem {count, plural, one {# mensagem} other {# mensagens}}}}",
	"quoted": "É '{assim}' que se escreve",
	"emptyMessage": "",
	"URLLabel": "Abrir {url}",
	"justName": "{name}"
}
"#;

const ARABIC: &str = r#"{
	"appTitle": "ملاحظات ميدانية",
	"cartItems": "{count, plural, zero {لا عناصر} one {عنصر واحد} two {عنصران} few {# عناصر} many {# عنصرًا} other {# عنصر}}",
	"distance": "{km, plural, =1 {كيلومتر واحد} other {# كيلومترات}} متبقية"
}
"#;

#[test]
fn generates_the_module_and_summaries() {
	let locales = Locales::new(&[
		("en.arb", ENGLISH),
		("pt-BR.arb", PORTUGUESE),
		("ar.arb", ARABIC),
		("README.md", "Only .arb files are locales."),
		("notes.txt", "{ not json, not read"),
	]);
	let generated = leptoskit_l10n_codegen::generate(&locales.dir(), "en").expect("valid files");
	insta::assert_snapshot!(generated.source);

	assert_eq!(
		generated.locales,
		[
			LocaleEntry {
				tag: "en".to_owned(),
				file: "en.arb".to_owned(),
				template: true,
				untranslated: vec![],
			},
			LocaleEntry {
				tag: "ar".to_owned(),
				file: "ar.arb".to_owned(),
				template: false,
				untranslated: [
					"emptyMessage",
					"greeting",
					"inbox",
					"justName",
					"pronoun",
					"quoted",
					"URLLabel",
				]
				.map(String::from)
				.to_vec(),
			},
			LocaleEntry {
				tag: "pt-BR".to_owned(),
				file: "pt-BR.arb".to_owned(),
				template: false,
				untranslated: vec!["distance".to_owned()],
			},
		]
	);
	let signatures: Vec<String> = generated
		.messages
		.iter()
		.map(|message| format!("{}: {}", message.key, message.signature))
		.collect();
	assert_eq!(
		signatures,
		[
			"appTitle: app_title() -> &'static str",
			"cartItems: cart_items(count: i64) -> String",
			"distance: distance(km: f64) -> String",
			"emptyMessage: empty_message() -> &'static str",
			"greeting: greeting(name: &str) -> String",
			"inbox: inbox(who: &str, count: i64) -> String",
			"justName: just_name(name: &str) -> String",
			"pronoun: pronoun(gender: &str) -> String",
			"quoted: quoted() -> &'static str",
			"URLLabel: url_label(url: &str) -> String",
		]
	);
}

#[test]
fn reports_every_problem_with_its_file_and_line() {
	let locales = Locales::new(&[
		(
			"en.arb",
			r#"{
	"@@locale": "fr",
	"appTitle": "Field Notes",
	"appTitle": "Twice",
	"type": "A keyword",
	"9lives": "Starts with a digit",
	"---": "Nothing left",
	"helloWorld": "one",
	"hello_world": "the same method",
	"unclosed": "Hello, {name",
	"notText": 1,
	"@orphan": {},
	"@appTitle": "not an object",
	"described": "x",
	"@described": {
		"description": 1,
		"placeholders": []
	},
	"typed": "{when}",
	"@typed": {
		"placeholders": {
			"when": { "type": "DateTime" },
			"ghost": {}
		}
	},
	"untyped": "{thing}",
	"@untyped": {
		"placeholders": {
			"thing": "String",
			"other": { "type": 1 }
		}
	},
	"both": "{x, plural, other {{x, select, other {}}}}",
	"pluralOfString": "{n, plural, other {}}",
	"@pluralOfString": { "placeholders": { "n": { "type": "String" } } },
	"selectOfInt": "{n, select, other {}}",
	"@selectOfInt": { "placeholders": { "n": { "type": "int" } } },
	"keywordParam": "{type}",
	"sameParam": "{userName} {user_name}"
}
"#,
		),
		(
			"pt-BR.arb",
			r#"{
	"@@locale": "pt-BR",
	"extra": "Not in the template",
	"appTitle": "{surprise}",
	"typed": "{when} {other}",
	"cartItems": "{n}",
	"described": "{x, plural, other {}}",
	"unclosed": "{name, plural, other {}}",
	"both": "{x}"
}
"#,
		),
		("es.arb", "{ not json"),
		("fr.arb", "[]"),
		("messages.arb", "{}"),
		("not a locale.arb", "{}"),
	]);
	insta::assert_snapshot!(locales.diagnostics("en"));
}

#[test]
fn reports_a_missing_or_empty_template() {
	let locales = Locales::new(&[("pt-BR.arb", "{}")]);
	assert_eq!(
		locales.diagnostics("en"),
		"locales: there is no `en.arb`, the template every other file translates; add it, or set \
		 `[l10n] template` to the locale of the file that is"
	);
	assert_eq!(
		locales.diagnostics("pt-br"),
		"locales/pt-BR.arb: the template has no messages; add one, such as `\"appTitle\": \"My \
		 App\"`"
	);
	assert_eq!(
		locales.diagnostics("not a tag"),
		"locales: `not a tag` is not a language tag; set `[l10n] template` to the locale of the \
		 file every other file translates, such as \"en\""
	);

	let empty = Locales::new(&[]);
	assert_eq!(
		empty.diagnostics("en"),
		"locales: there are no `.arb` files; add `en.arb` with a message such as `\"appTitle\": \
		 \"My App\"`"
	);

	let missing = empty.project.path().join("nowhere");
	let diagnostics =
		leptoskit_l10n_codegen::generate(&missing, "en").expect_err("the directory is missing");
	assert_eq!(
		diagnostics.to_string(),
		format!(
			"{}: the locales directory does not exist; create it with `en.arb` inside",
			missing.display()
		)
	);
	let not_a_dir = empty.project.path().join("l10n.rs");
	std::fs::write(&not_a_dir, "").expect("a file");
	assert!(
		leptoskit_l10n_codegen::generate(&not_a_dir, "en")
			.unwrap_err()
			.to_string()
			.contains("does not exist")
	);
}

#[test]
fn writes_checks_and_never_overwrites_a_hand_written_module() {
	let locales = Locales::new(&[("en.arb", ENGLISH), ("pt-BR.arb", PORTUGUESE)]);
	let output = locales.output();
	let generated = leptoskit_l10n_codegen::generate(&locales.dir(), "en").expect("valid files");

	assert_eq!(
		leptoskit_l10n_codegen::status(&output, &generated).unwrap(),
		Status::Missing
	);
	assert!(leptoskit_l10n_codegen::write(&output, &generated).unwrap());
	assert_eq!(
		leptoskit_l10n_codegen::status(&output, &generated).unwrap(),
		Status::Current
	);
	assert!(!leptoskit_l10n_codegen::write(&output, &generated).unwrap());

	// Formatting the file keeps it current; a change to the messages does not.
	let written = std::fs::read_to_string(&output).unwrap();
	let reformatted = written.replace("    ", "\t").replace(" {\n", "\n{\n");
	assert_ne!(written, reformatted);
	std::fs::write(&output, reformatted).unwrap();
	assert_eq!(
		leptoskit_l10n_codegen::status(&output, &generated).unwrap(),
		Status::Current
	);
	locales.write("pt-BR.arb", &PORTUGUESE.replace("Olá", "Oi"));
	let changed = leptoskit_l10n_codegen::generate(&locales.dir(), "en").expect("valid files");
	assert_eq!(
		leptoskit_l10n_codegen::status(&output, &changed).unwrap(),
		Status::Stale
	);
	assert!(leptoskit_l10n_codegen::write(&output, &changed).unwrap());
	assert!(
		std::fs::read_to_string(&output)
			.unwrap()
			.contains("Oi, {name}!")
	);

	// A file leptoskit did not write stays as it is.
	std::fs::write(&output, "pub struct Mine;\n").unwrap();
	let error = leptoskit_l10n_codegen::write(&output, &changed).unwrap_err();
	assert!(matches!(&error, WriteError::NotGenerated(path) if path == &output));
	assert!(error.to_string().contains("was not generated by leptoskit"));
	assert_eq!(
		std::fs::read_to_string(&output).unwrap(),
		"pub struct Mine;\n"
	);

	// A path that cannot be read is an error, not a missing file.
	let directory = locales.project.path().join("a_directory");
	std::fs::create_dir(&directory).unwrap();
	let error = leptoskit_l10n_codegen::status(&directory, &changed).unwrap_err();
	assert!(matches!(&error, WriteError::Io { path, .. } if path == &directory));
	assert!(error.to_string().contains("could not access"));
	assert!(std::error::Error::source(&error).is_some());
}

#[test]
fn diagnostics_render_with_their_file_and_line() {
	let locales = Locales::new(&[("en.arb", "{\n\t\"bad\": \"{\"\n}\n")]);
	let diagnostics = leptoskit_l10n_codegen::generate(&locales.dir(), "en").unwrap_err();
	let [diagnostic] = diagnostics.0.as_slice() else {
		panic!("{diagnostics}");
	};
	assert_eq!(diagnostic.file, locales.dir().join("en.arb"));
	assert_eq!(diagnostic.line, Some(2));
	assert_eq!(
		diagnostic.relative_to(locales.project.path()),
		"locales/en.arb:2: in `bad`, a placeholder needs a name that starts with a letter or `_`: \
		 `{name}` (at character 2)"
	);
	assert_eq!(
		diagnostic.to_string(),
		format!(
			"{}: {}",
			locales.dir().join("en.arb").display(),
			"in `bad`, a placeholder needs a name that starts with a letter or `_`: `{name}` (at \
			 character 2)"
		)
		.replacen(": in", ":2: in", 1)
	);
	assert_eq!(
		diagnostic.relative_to(Path::new("/elsewhere")),
		diagnostic.to_string()
	);
}
