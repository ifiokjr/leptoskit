//! Typed translations for leptoskit, in the style of Flutter's `gen_l10n`.
//!
//! Messages live in one ARB file per locale (`locales/en.arb`,
//! `locales/pt-BR.arb`), and `leptoskit l10n` writes `src/l10n.rs` from
//! them: a `Messages` trait with one method per message, typed by the
//! message's placeholders; one implementation per locale; and the glue this
//! crate needs. An app then reads its text through a reactive [`L10n`]
//! handle, and a view that reads a message renders again when the locale
//! changes:
//!
//! ```ignore
//! let l10n = use_l10n();
//! view! { <Text>{move || l10n.items_in_cart(count.get())}</Text> }
//! ```
//!
//! This crate is the runtime behind that module: [`Locale`] tags, the
//! [`negotiate`] step that picks a supported locale for a user,
//! [`PluralRules`] for messages that count, and the [`L10n`] handle that
//! provides the translations to a component tree. The localization guide
//! covers the setup; this page covers the parts.
//!
//! # A translations module by hand
//!
//! The generated module is ordinary Rust. This is its shape, written by hand
//! for two locales, so the mechanics are visible:
//!
//! ```
//! use std::sync::LazyLock;
//!
//! use leptoskit_l10n::L10n;
//! use leptoskit_l10n::Locale;
//! use leptoskit_l10n::PluralCategory;
//! use leptoskit_l10n::PluralRules;
//! use leptoskit_l10n::Translations;
//! use reactive_graph::owner::Owner;
//!
//! /// One method per message, typed by its placeholders.
//! pub trait Messages: Send + Sync {
//! 	fn greeting(&self, name: &str) -> String;
//! 	fn items(&self, count: i64) -> String;
//! }
//!
//! pub struct En;
//!
//! impl Messages for En {
//! 	fn greeting(&self, name: &str) -> String {
//! 		format!("Hello, {name}!")
//! 	}
//!
//! 	fn items(&self, count: i64) -> String {
//! 		static RULES: LazyLock<PluralRules> =
//! 			LazyLock::new(|| PluralRules::for_locale(&Locale::new("en").unwrap()));
//! 		match RULES.category(count) {
//! 			PluralCategory::One => format!("{count} item"),
//! 			_ => format!("{count} items"),
//! 		}
//! 	}
//! }
//!
//! pub struct PtBr;
//!
//! impl Messages for PtBr {
//! 	fn greeting(&self, name: &str) -> String {
//! 		format!("Olá, {name}!")
//! 	}
//!
//! 	fn items(&self, count: i64) -> String {
//! 		static RULES: LazyLock<PluralRules> =
//! 			LazyLock::new(|| PluralRules::for_locale(&Locale::new("pt-BR").unwrap()));
//! 		match RULES.category(count) {
//! 			PluralCategory::One => format!("{count} item"),
//! 			_ => format!("{count} itens"),
//! 		}
//! 	}
//! }
//!
//! /// Which locale has which messages.
//! pub struct AppTranslations;
//!
//! impl Translations for AppTranslations {
//! 	type Messages = dyn Messages;
//!
//! 	fn supported_locales() -> &'static [Locale] {
//! 		static SUPPORTED: LazyLock<Vec<Locale>> =
//! 			LazyLock::new(|| vec![Locale::new("en").unwrap(), Locale::new("pt-BR").unwrap()]);
//! 		&SUPPORTED
//! 	}
//!
//! 	fn messages(locale: &Locale) -> &'static dyn Messages {
//! 		match locale.language() {
//! 			"pt" => &PtBr,
//! 			_ => &En,
//! 		}
//! 	}
//! }
//!
//! // Where the app starts: negotiate once, and provide the handle to the tree.
//! Owner::new().with(|| {
//! 	let preferred = [Locale::new("pt-PT").unwrap()];
//! 	let l10n = L10n::<AppTranslations>::new(&preferred);
//! 	l10n.provide();
//!
//! 	// In any component below.
//! 	let l10n: L10n<AppTranslations> = leptoskit_l10n::use_l10n();
//! 	assert_eq!(l10n.locale(), "pt-BR");
//! 	assert_eq!(l10n.greeting("Ana"), "Olá, Ana!");
//! 	assert_eq!(l10n.items(1), "1 item");
//!
//! 	// From a language menu.
//! 	l10n.set_locale(&Locale::new("en").unwrap());
//! 	assert_eq!(l10n.items(2), "2 items");
//! });
//! ```
//!
//! # Where the preferred locales come from
//!
//! - In the browser, [`browser_locales`] (behind the `web` feature) reads
//!   `navigator.languages`.
//! - On the server, [`accept_language`] reads the request's `Accept-Language`
//!   header, so the page renders in the language the browser will hydrate it
//!   in.
//! - In a native app, the `localization` runtime plugin reports the device's
//!   locales; parse their tags with [`Locale::new`].
//! - An empty list starts on the template locale, and [`L10n::set_locale`]
//!   switches later, from a language menu or a saved preference.
//!
//! # Plurals
//!
//! A message such as `{count, plural, one {# item} other {# items}}` picks
//! its form through [`PluralRules`], which carry CLDR's cardinal rules for
//! every language it covers. Messages name the categories a language uses
//! and always name `other`; the generated code asks the rules which one a
//! number takes, so a Polish message can write `one`, `few`, `many`, and
//! `other` and get each at the right numbers.
#![deny(missing_docs)]
#![doc(
	html_logo_url = "https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/icon.svg"
)]
#![doc(
	html_favicon_url = "https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg"
)]

mod handle;
mod locale;
mod negotiate;
mod plural;
#[cfg(feature = "web")]
mod web;

pub use handle::L10n;
pub use handle::Translations;
pub use handle::use_l10n;
pub use locale::Locale;
pub use locale::LocaleError;
pub use locale::accept_language;
pub use negotiate::negotiate;
pub use plural::PluralCategory;
pub use plural::PluralNumber;
pub use plural::PluralRules;
#[cfg(feature = "web")]
pub use web::browser_locales;

/// Support for the code `leptoskit l10n` generates. Not public API.
#[doc(hidden)]
pub mod __private {
	/// A locale from a tag the generator already validated.
	#[must_use]
	pub fn locale(tag: &str) -> crate::Locale {
		crate::Locale::new(tag).expect("`leptoskit l10n` validated every tag it wrote")
	}
}
