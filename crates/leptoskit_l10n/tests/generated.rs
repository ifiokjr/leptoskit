//! The module `leptoskit l10n` writes, compiled against this crate and run:
//! the proof that the generated code and the runtime agree.

use std::path::Path;

use leptoskit_l10n::Locale;
use reactive_graph::owner::Owner;

#[path = "fixtures/l10n.rs"]
mod l10n;

fn locale(tag: &str) -> Locale {
	Locale::new(tag).unwrap()
}

#[test]
fn the_committed_module_is_what_the_generator_writes() {
	let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
	let generated = leptoskit_l10n_codegen::generate(&fixtures.join("locales"), "en")
		.expect("the fixture files are valid");
	assert_eq!(
		leptoskit_l10n_codegen::status(&fixtures.join("l10n.rs"), &generated).unwrap(),
		leptoskit_l10n_codegen::Status::Current,
		"tests/fixtures/l10n.rs is out of date with tests/fixtures/locales; regenerate it"
	);
}

/// Runs `read` with the translations started from `tag` and provided, as an
/// app does where it starts.
fn reading(tag: &str, read: impl FnOnce(l10n::L10n)) {
	Owner::new().with(|| {
		l10n::L10n::new(&[locale(tag)]).provide();
		read(l10n::use_l10n());
	});
}

#[test]
fn brazilian_portuguese_serves_portugal_and_falls_back_where_untranslated() {
	reading("pt-PT", |l10n| {
		assert_eq!(l10n.locale(), "pt-BR");
		assert_eq!(l10n.app_title(), "Notas de Campo");
		assert_eq!(l10n.greeting("Ana"), "Olá, Ana!");
		assert_eq!(l10n.cart_items(0), "Seu carrinho está vazio");
		assert_eq!(l10n.cart_items(1), "1 item no carrinho");
		assert_eq!(l10n.cart_items(2), "2 itens no carrinho");
		assert_eq!(l10n.pronoun("female"), "ela");
		assert_eq!(l10n.pronoun("unknown"), "elu");
		assert_eq!(l10n.inbox("you", 1), "Você tem 1 mensagem");
		assert_eq!(l10n.inbox("Ana", 3), "Ana tem 3 mensagens");
		assert_eq!(l10n.quoted(), "É {assim} que se escreve");
		assert_eq!(l10n.empty_message(), "");
		assert_eq!(l10n.url_label("x"), "Abrir x");
		assert_eq!(l10n.just_name("Ana"), "Ana");
		// `distance` is not translated in pt-BR, so the template's text shows.
		assert_eq!(l10n.distance(1.0), "one kilometre to go");
		assert_eq!(l10n.distance(2.5), "2.5 kilometres to go");
	});
}

#[test]
fn arabic_counts_with_its_own_forms() {
	reading("ar", |l10n| {
		assert_eq!(l10n.locale(), "ar");
		assert_eq!(l10n.app_title(), "ملاحظات ميدانية");
		assert_eq!(l10n.cart_items(0), "لا عناصر");
		assert_eq!(l10n.cart_items(1), "عنصر واحد");
		assert_eq!(l10n.cart_items(2), "عنصران");
		assert_eq!(l10n.cart_items(3), "3 عناصر");
		assert_eq!(l10n.cart_items(11), "11 عنصرًا");
		assert_eq!(l10n.cart_items(100), "100 عنصر");
		assert_eq!(l10n.distance(1.0), "كيلومتر واحد متبقية");
		assert_eq!(l10n.distance(3.0), "3 كيلومترات متبقية");
		assert_eq!(l10n.greeting("Ana"), "Hello, Ana!");
		assert_eq!(l10n.inbox("you", 2), "You have 2 messages");
	});
}

#[test]
fn english_is_the_template_and_lists_every_locale() {
	reading("en-GB", |l10n| {
		assert_eq!(l10n.locale(), "en");
		assert_eq!(l10n.cart_items(0), "Your cart is empty");
		assert_eq!(l10n.cart_items(1), "1 item in your cart");
		assert_eq!(l10n.cart_items(21), "21 items in your cart");
		assert_eq!(l10n.inbox("Ana", 1), "Ana has 1 message");
		assert_eq!(
			l10n.quoted(),
			"It's {not a placeholder} with a lone ' and `backticks`"
		);
		let supported: Vec<String> = l10n
			.supported_locales()
			.iter()
			.map(ToString::to_string)
			.collect();
		assert_eq!(
			supported,
			["en", "ar", "ca-ES", "ca-ES-valencia", "ja", "pl", "pt-BR"]
		);
	});
}

#[test]
fn switching_the_locale_rereads_every_message() {
	reading("pt-PT", |l10n| {
		assert_eq!(l10n.app_title(), "Notas de Campo");
		l10n.set_locale(&locale("ar"));
		assert_eq!(l10n.locale(), "ar");
		assert_eq!(l10n.app_title(), "ملاحظات ميدانية");
		l10n.set_locale(&locale("en-GB"));
		assert_eq!(l10n.locale(), "en");
		assert_eq!(l10n.cart_items(1), "1 item in your cart");
	});
}

#[test]
fn template_shapes_compile_and_read_right() {
	reading("en", |l10n| {
		// An empty form is an empty string.
		assert_eq!(l10n.remaining(0), "");
		assert_eq!(l10n.remaining(3), "3 left");
		// Two plurals over one placeholder bind two locals.
		assert_eq!(l10n.two_plurals(1), "1 apple and 1 pear");
		assert_eq!(l10n.two_plurals(2), "2 apples and 2 pears");
		// A placeholder named like the local a plural binds keeps its name.
		assert_eq!(l10n.due_in("Rent", 1), "Rent: 1 day");
		assert_eq!(l10n.due_in("Rent", 5), "Rent: 5 days");
		// `#` counts the innermost plural.
		assert_eq!(l10n.nested(1, 2), "2 bs for one a");
		assert_eq!(l10n.nested(3, 1), "1 b for 3 as");
		// Exact float forms inside a select.
		assert_eq!(l10n.run("run", 1.0), "a one-kilometre run");
		assert_eq!(l10n.run("run", 2.5), "a 2.5 km run");
		assert_eq!(l10n.run("walk", 0.0), "nowhere");
		assert_eq!(l10n.run("walk", 4.0), "4 km");

		// `=N` forms with one text share a branch, and join a category with
		// it; an `=N` form with the text of `other` is `other`.
		assert_eq!(l10n.merged(0), "none");
		assert_eq!(l10n.merged(2), "none");
		assert_eq!(l10n.merged(1), "just one");
		assert_eq!(l10n.merged(3), "3 of them");
		assert_eq!(l10n.same_as_other(1), "1 left");
		assert_eq!(l10n.same_as_other(4), "4 left");
	});
}

#[test]
fn polish_names_one_and_few_and_merges_forms_that_read_alike() {
	reading("pl", |l10n| {
		// Polish names `one` and `few`; `many` falls to `other`.
		assert_eq!(l10n.cart_items(0), "Koszyk jest pusty");
		assert_eq!(l10n.cart_items(1), "1 przedmiot w koszyku");
		assert_eq!(l10n.cart_items(3), "3 przedmioty w koszyku");
		assert_eq!(l10n.cart_items(5), "5 przedmiotów w koszyku");
		assert_eq!(l10n.cart_items(22), "22 przedmioty w koszyku");
		// `few` reads like `other` and is left to it; `female` and `male`
		// share one branch.
		assert_eq!(l10n.photos_taken(3), "3 zdjęcia morza");
		assert_eq!(l10n.photos_taken(5), "5 zdjęć morza");
		assert_eq!(l10n.sign_off("female"), "Ściska was mocno");
		assert_eq!(l10n.sign_off("male"), "Ściska was mocno");
		assert_eq!(l10n.sign_off("other"), "Ściskają was mocno");
	});
}

#[test]
fn repeated_forms_share_an_arm_and_only_other_is_plain_text() {
	// Arabic writes `many` and `other` alike: one arm, `Many | Other`.
	reading("ar", |l10n| {
		assert_eq!(l10n.photos_taken(2), "صورتان للبحر");
		assert_eq!(l10n.photos_taken(11), "11 صورة للبحر");
		assert_eq!(l10n.photos_taken(100), "100 صورة للبحر");
	});

	// A select or plural with only `other` is its text, and leaves its
	// placeholder unread.
	reading("ja", |l10n| {
		assert_eq!(l10n.sign_off("female"), "愛をこめて");
		assert_eq!(l10n.cart_heading(1), "商品");
		assert_eq!(l10n.photos_taken(0), "海の写真はまだありません");
		assert_eq!(l10n.photos_taken(3), "海の写真が3枚");
	});
}

#[test]
fn locales_with_variants_get_their_own_messages() {
	Owner::new().with(|| {
		let l10n = l10n::L10n::new(&[locale("ca-ES-valencia")]);
		assert_eq!(l10n.locale(), "ca-ES-valencia");
		assert_eq!(l10n.cart_items(0), "La teua cistella està buida");
		assert_eq!(l10n.cart_items(1), "1 article en la cistella");
		assert_eq!(l10n.cart_items(2), "2 articles en la cistella");

		// Before, `ca-ES` and `ca-ES-valencia` matched one `(language, script,
		// region)` arm, so one of them could never be reached.
		l10n.set_locale(&locale("ca-ES"));
		assert_eq!(l10n.cart_items(0), "La teva cistella està buida");
		assert_eq!(l10n.cart_items(1), "1 article a la cistella");

		// A locale outside the list gets the template's messages.
		assert_eq!(
			<l10n::AppTranslations as leptoskit_l10n::Translations>::messages(&locale("ca-FR"))
				.cart_items(0),
			"Your cart is empty"
		);
	});
}

#[test]
fn browser_locales_are_empty_outside_a_browser() {
	// Exported with or without the `web` feature, so shared code compiles on
	// every platform.
	assert!(leptoskit_l10n::browser_locales().is_empty());
}
