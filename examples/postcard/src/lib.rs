//! Postcard, a card home from a trip in five languages: the worked example
//! for typed translations.
//!
//! The messages live in `locales/*.arb`, one file per language, and
//! `leptoskit l10n` writes [`l10n`] from them: a method per message, typed by
//! its placeholders. English is the template; Polish counts with four plural
//! forms, Arabic with six and right to left, Japanese with one, and Brazilian
//! Portuguese is half translated, so its missing messages show the English.
//!
//! [`PostcardApp`] owns the reactive scope the translations live in and
//! renders the [`screen`]; the tests switch languages, drag the counts, and
//! read what each language says.
//!
//! ```
//! use leptoskit_l10n::Locale;
//! use postcard_example::PostcardApp;
//!
//! let app = PostcardApp::new(&[Locale::new("pl").unwrap()]);
//! assert_eq!(app.l10n().app_title(), "Pocztówka");
//! assert_eq!(app.l10n().days_away(22), "22 dni poza domem");
//! ```

pub mod l10n;
pub mod screen;
pub mod trip;

mod app;

pub use app::PostcardApp;
pub use app::VIEWPORT;
