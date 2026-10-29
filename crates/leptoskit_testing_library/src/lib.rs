//! Testing Library queries and widget-test actions for leptoskit components
//! in a real browser.
//!
//! [`render`] mounts a component with the leptoskit web renderer, the same
//! way a leptoskit web app does, and returns a [`Screen`]. Find what a user
//! would find, by text, label, role, or test identifier, act on it, and
//! assert on what changed. Tests run in Chrome or Firefox through
//! `wasm-bindgen-test`, so they see the DOM your users get.
//!
//! ```
//! # #[cfg(target_arch = "wasm32")]
//! # mod example {
//! use leptos::prelude::Get;
//! use leptos::prelude::RwSignal;
//! use leptos::prelude::Update;
//! use leptoskit::prelude::Button;
//! use leptoskit::prelude::Element;
//! use leptoskit::prelude::Text;
//! use leptoskit::prelude::View;
//! use leptoskit::prelude::component;
//! use leptoskit::view;
//! use leptoskit_testing_library::Finder;
//! use leptoskit_testing_library::Queries;
//! use leptoskit_testing_library::Role;
//! use leptoskit_testing_library::TestError;
//! use leptoskit_testing_library::render;
//! use wasm_bindgen_test::*;
//!
//! wasm_bindgen_test_configure!(run_in_browser);
//!
//! #[component]
//! fn Counter(count: RwSignal<u32>) -> Element {
//! 	view! {
//! 		<View>
//! 			<Text>{format!("Count: {}", count.get())}</Text>
//! 			<Button on:press=move || count.update(|count| *count += 1)>"Increment"</Button>
//! 		</View>
//! 	}
//! }
//!
//! #[wasm_bindgen_test]
//! async fn increments() -> Result<(), TestError> {
//! 	// State lives outside the view, which re-renders when it changes.
//! 	let count = RwSignal::new(0);
//! 	let screen = render(move || view! { <Counter count=count /> });
//!
//! 	screen.get_by_text("Count: 0")?;
//! 	screen
//! 		.tap(&Finder::aria_role(Role::Button, "Increment"))
//! 		.await?;
//! 	screen.get_by_text("Count: 1")?;
//! 	assert!(screen.query_by_text("Count: 0")?.is_none());
//! 	Ok(())
//! }
//! # }
//! ```
//!
//! # Queries
//!
//! [`Screen`] implements [`Queries`], so it has Testing Library's query
//! families, each by text, label, role, and test identifier:
//!
//! - `get_by_*` returns the one match, and fails on none or several.
//! - `get_all_by_*` returns every match, and fails on none.
//! - `query_by_*` returns `Ok(None)` for no match, for asserting absence.
//! - `query_all_by_*` returns every match, possibly none.
//! - `find_by_*` and [`Screen::find_all_by`] wait for a match to appear,
//!   pumping leptos until it does or the timeout passes (one second by default;
//!   see [`Screen::with_timeout`]).
//!
//! Hidden elements never match, and every failure is a [`TestError`] that
//! shows the rendered markup.
//!
//! # Widget-test actions
//!
//! On top of the queries, [`Screen::tap`], [`Screen::enter_text`], and
//! [`Screen::toggle`] act like a user: they find exactly one enabled
//! control, dispatch the browser events a click or typing would, and
//! [`Screen::pump`] leptos so the next assertion sees the result.
//!
//! # Testing Library and widget tests
//!
//! The vocabulary is shared with `leptoskit_testing`. A [`Finder`] describes
//! what to look for, and [`Queries`] turns it into an assertion, whether the
//! component is rendered headless with `leptoskit_testing::WidgetTester`
//! (native, no browser, fast, with layout and hit testing) or in a browser
//! with [`Screen`] (real DOM, real events, real CSS). The same
//! `get_by_role(Role::Button, "Increment")` works on both; only `render`
//! and the awaits differ.
//!
//! Browser code only runs on `wasm32-unknown-unknown`. The crate still
//! compiles on other targets so a workspace-wide check covers it.
#![doc(
	html_logo_url = "https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/icon.svg"
)]
#![doc(
	html_favicon_url = "https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg"
)]

mod dom;
mod screen;

pub use leptoskit_native::NativeKind;
pub use leptoskit_native::Role;
pub use leptoskit_testing::Finder;
pub use leptoskit_testing::Queries;
pub use leptoskit_testing::TestError;
pub use screen::DEFAULT_TIMEOUT;
pub use screen::Screen;
pub use screen::render;
