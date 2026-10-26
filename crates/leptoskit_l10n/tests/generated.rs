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

#[test]
fn messages_read_in_the_locale_in_use() {
	Owner::new().with(|| {
		let provided = l10n::L10n::new(&[locale("pt-PT")]);
		provided.provide();
		let l10n = l10n::use_l10n();
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

		l10n.set_locale(&locale("ar"));
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

		l10n.set_locale(&locale("en-GB"));
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
		assert_eq!(supported, ["en", "ar", "pt-BR"]);
	});
}
