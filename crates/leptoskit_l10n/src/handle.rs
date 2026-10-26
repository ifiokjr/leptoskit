//! The reactive handle an app reads its translations through.

use std::fmt;
use std::marker::PhantomData;
use std::ops::Deref;

use reactive_graph::owner::provide_context;
use reactive_graph::owner::use_context;
use reactive_graph::signal::RwSignal;
use reactive_graph::traits::Get;
use reactive_graph::traits::Set;
use reactive_graph::traits::With;
use reactive_graph::traits::WithUntracked;

use crate::Locale;
use crate::negotiate;

/// What a generated translations module provides: the locales it has
/// messages for, and the messages of each.
///
/// `leptoskit l10n` writes an implementation from an app's ARB files. The
/// crate documentation writes one by hand to show the mechanics; an app
/// never needs to.
pub trait Translations: 'static {
	/// The messages of one locale: the generated `Messages` trait, as
	/// `dyn Messages`.
	type Messages: ?Sized + 'static;

	/// Every locale with messages, the template locale first. Never empty.
	fn supported_locales() -> &'static [Locale];

	/// The messages of one of the supported locales.
	fn messages(locale: &Locale) -> &'static Self::Messages;
}

/// An app's translations, for the locale in use.
///
/// Create one where the app starts, from the locales the user prefers, and
/// call [`provide`](Self::provide) so that [`use_l10n`] finds it in every
/// component below. The handle is `Copy`. Every message read goes through
/// the active locale's signal, so a view that reads a message renders again
/// when the locale changes.
pub struct L10n<T: Translations> {
	locale: RwSignal<Locale>,
	translations: PhantomData<fn() -> T>,
}

impl<T: Translations> L10n<T> {
	/// Starts on the supported locale that best serves `preferred` (see
	/// [`negotiate`]), or on the template locale when none does.
	#[must_use]
	pub fn new(preferred: &[Locale]) -> Self {
		Self {
			locale: RwSignal::new(Self::resolve(preferred)),
			translations: PhantomData,
		}
	}

	fn resolve(preferred: &[Locale]) -> Locale {
		let supported = T::supported_locales();
		negotiate(preferred, supported)
			.or_else(|| supported.first())
			.expect("Translations::supported_locales() lists the template locale first")
			.clone()
	}

	/// The locale in use. Reading it inside a reactive scope tracks it.
	#[must_use]
	pub fn locale(&self) -> Locale {
		self.locale.get()
	}

	/// Switches to the supported locale that best serves `locale`, or to the
	/// template locale when none does. Nothing happens when that is the
	/// locale already in use.
	pub fn set_locale(&self, locale: &Locale) {
		let next = Self::resolve(std::slice::from_ref(locale));
		if self.locale.with_untracked(|current| *current != next) {
			self.locale.set(next);
		}
	}

	/// Every locale with messages, the template locale first: what a
	/// language menu lists.
	#[must_use]
	pub fn supported_locales(&self) -> &'static [Locale] {
		T::supported_locales()
	}

	/// The messages of the locale in use. The handle also dereferences to
	/// them, so `l10n.greeting()` reads as `l10n.messages().greeting()`.
	#[must_use]
	pub fn messages(&self) -> &'static T::Messages {
		self.locale.with(T::messages)
	}

	/// Shares the handle with every component below, where [`use_l10n`]
	/// returns it.
	pub fn provide(&self) {
		provide_context(*self);
	}
}

impl<T: Translations> Clone for L10n<T> {
	fn clone(&self) -> Self {
		*self
	}
}

impl<T: Translations> Copy for L10n<T> {}

impl<T: Translations> fmt::Debug for L10n<T> {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter
			.debug_struct("L10n")
			.field("locale", &self.locale.with_untracked(ToString::to_string))
			.finish()
	}
}

impl<T: Translations> Deref for L10n<T> {
	type Target = T::Messages;

	fn deref(&self) -> &T::Messages {
		self.messages()
	}
}

/// The translations an ancestor component provided.
///
/// # Panics
///
/// When no ancestor called [`L10n::provide`].
#[must_use]
#[track_caller]
pub fn use_l10n<T: Translations>() -> L10n<T> {
	use_context::<L10n<T>>().expect(
		"no translations in context: call `L10n::new(&preferred).provide()` in an ancestor \
		 component",
	)
}

#[cfg(test)]
mod tests {
	use std::sync::LazyLock;

	use reactive_graph::computed::ArcMemo;
	use reactive_graph::owner::Owner;

	use super::*;

	trait Greeting: Send + Sync {
		fn hello(&self) -> &'static str;
	}

	struct En;

	impl Greeting for En {
		fn hello(&self) -> &'static str {
			"Hello"
		}
	}

	struct Pt;

	impl Greeting for Pt {
		fn hello(&self) -> &'static str {
			"Olá"
		}
	}

	struct App;

	impl Translations for App {
		type Messages = dyn Greeting;

		fn supported_locales() -> &'static [Locale] {
			static SUPPORTED: LazyLock<Vec<Locale>> =
				LazyLock::new(|| vec![Locale::new("en").unwrap(), Locale::new("pt-BR").unwrap()]);
			&SUPPORTED
		}

		fn messages(locale: &Locale) -> &'static dyn Greeting {
			if locale.language() == "pt" { &Pt } else { &En }
		}
	}

	fn locale(tag: &str) -> Locale {
		Locale::new(tag).unwrap()
	}

	#[test]
	fn starts_on_the_best_match_and_falls_back_to_the_template() {
		Owner::new().with(|| {
			let l10n = L10n::<App>::new(&[locale("pt-PT")]);
			assert_eq!(l10n.locale(), "pt-BR");
			assert_eq!(l10n.hello(), "Olá");
			assert_eq!(l10n.messages().hello(), "Olá");

			let l10n = L10n::<App>::new(&[locale("de"), locale("fr")]);
			assert_eq!(l10n.locale(), "en");
			assert_eq!(l10n.hello(), "Hello");
			assert_eq!(l10n.supported_locales(), App::supported_locales());
			assert_eq!(format!("{l10n:?}"), "L10n { locale: \"en\" }");
		});
	}

	#[test]
	fn message_reads_follow_the_locale() {
		Owner::new().with(|| {
			let l10n = L10n::<App>::new(&[]);
			let greeting = ArcMemo::new(move |_| l10n.hello());
			let tag = ArcMemo::new(move |_| l10n.locale().to_string());
			assert_eq!(greeting.get(), "Hello");
			assert_eq!(tag.get(), "en");

			l10n.set_locale(&locale("pt"));
			assert_eq!(greeting.get(), "Olá");
			assert_eq!(tag.get(), "pt-BR");

			// An unsupported locale means the template, and a locale already in
			// use changes nothing.
			l10n.set_locale(&locale("ja"));
			assert_eq!(tag.get(), "en");
			l10n.set_locale(&locale("en-GB"));
			assert_eq!(tag.get(), "en");
		});
	}

	#[test]
	fn provided_handles_are_found_below() {
		Owner::new().with(|| {
			let l10n = L10n::<App>::new(&[locale("pt-BR")]);
			l10n.provide();
			let copy = l10n;
			let found: L10n<App> = use_l10n();
			assert_eq!(found.locale(), copy.clone().locale());
			found.set_locale(&locale("en"));
			assert_eq!(l10n.locale(), "en");
		});
	}

	#[test]
	#[should_panic(expected = "no translations in context")]
	fn a_missing_provider_says_what_to_do() {
		Owner::new().with(|| {
			let _ = use_l10n::<App>();
		});
	}
}
