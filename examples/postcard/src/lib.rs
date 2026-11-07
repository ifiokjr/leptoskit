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

/// The language a page's query string names with `lang`, as in `?lang=ar`.
///
/// ```
/// assert_eq!(
/// 	postcard_example::language_in_query("?lang=ar").as_deref(),
/// 	Some("ar")
/// );
/// assert_eq!(
/// 	postcard_example::language_in_query("?a=1&lang=pt-BR").as_deref(),
/// 	Some("pt-BR")
/// );
/// assert!(postcard_example::language_in_query("").is_none());
/// ```
pub fn language_in_query(query: &str) -> Option<String> {
	query
		.trim_start_matches('?')
		.split('&')
		.find_map(|pair| pair.strip_prefix("lang="))
		.filter(|tag| !tag.is_empty())
		.map(str::to_owned)
}

/// Mounts the postcard when the browser loads the Wasm module, at the size
/// it is laid out for, in the language the page's query string names (such
/// as `?lang=ar`) or else the browser's own. `leptoskit dev` builds it with
/// the `csr` feature and serves it from `web/index.html`.
#[cfg(all(target_arch = "wasm32", feature = "csr"))]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
	use leptoskit::prelude::*;
	use leptoskit::yoga::Dimension;

	let named = web_sys::window()
		.and_then(|window| window.location().search().ok())
		.and_then(|query| language_in_query(&query))
		.and_then(|tag| leptoskit_l10n::Locale::new(&tag).ok());
	let preferred = match named {
		Some(locale) => vec![locale],
		None => leptoskit_l10n::browser_locales(),
	};

	leptos::mount::mount_to_body(move || {
		let app = PostcardApp::new(&preferred);
		move || {
			leptoskit::web::render(view! {
				<View with_style=|style| {
					style.width = Dimension::Points(VIEWPORT.0);
					style.height = Dimension::Points(VIEWPORT.1);
				}>
					{app.render()}
				</View>
			})
		}
	});
}
