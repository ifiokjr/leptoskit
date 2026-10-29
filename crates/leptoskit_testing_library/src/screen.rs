//! A mounted component, its queries, and the actions a user takes on it.

use std::time::Duration;

use leptos::mount::UnmountHandle;
use leptos::mount::mount_to;
use leptos::tachys::reactive_graph::RenderEffectState;
use leptos::tachys::view::any_view::AnyViewState;
use leptoskit_native::Element;
use leptoskit_native::Role;
use leptoskit_testing::Finder;
use leptoskit_testing::Queries;
use leptoskit_testing::TestError;
use wasm_bindgen::JsCast;
use web_sys::HtmlElement;
use web_sys::HtmlInputElement;
use web_sys::HtmlTextAreaElement;

use crate::dom;

/// How long the `find_by` family waits unless [`Screen::with_timeout`]
/// changes it.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(1);

/// How long a wait sleeps between queries, in milliseconds.
const POLL_INTERVAL_MS: i32 = 10;

/// Mounts `view` into a fresh `<div>` in the document body with the leptoskit
/// web renderer, the way a leptoskit web app mounts.
///
/// `view` runs once now and again whenever a signal it reads changes, so keep
/// state such as an `RwSignal` outside it and pass it in. Dropping the
/// returned [`Screen`] disposes the view's effects and removes the `<div>`,
/// so every test starts from a clean document.
///
/// # Panics
///
/// Panics outside a browser document, and for an element tree the web
/// renderer rejects (see `leptoskit_native::web::render`).
pub fn render(mut view: impl FnMut() -> Element + Send + 'static) -> Screen {
	let document = web_sys::window()
		.and_then(|window| window.document())
		.expect("render needs a browser document");
	let container = document
		.create_element("div")
		.expect("a document creates a div")
		.unchecked_into::<HtmlElement>();
	document
		.body()
		.expect("render needs a document body")
		.append_child(&container)
		.expect("the body accepts the test container");
	let mount = mount_to(container.clone(), move || {
		move || leptoskit_native::web::render(view())
	});

	Screen {
		container,
		mount: Some(mount),
		timeout: DEFAULT_TIMEOUT,
	}
}

/// A component mounted by [`render`]: query it through [`Queries`], wait for
/// it with the `find_by` family, and act on it like a user.
///
/// Actions dispatch real DOM events through the browser, then [`pump`] so
/// the assertions after them see the re-rendered DOM. They refuse disabled
/// controls and return a [`TestError`] showing the rendered markup on
/// failure, like `leptoskit_testing::WidgetTester`.
///
/// [`pump`]: Self::pump
#[must_use = "dropping a Screen unmounts it"]
pub struct Screen {
	container: HtmlElement,
	/// Taken in `drop` so the view unmounts while its container is attached.
	mount: Option<UnmountHandle<RenderEffectState<AnyViewState>>>,
	timeout: Duration,
}

impl Screen {
	/// The `<div>` the view is mounted in, for assertions the queries do not
	/// cover.
	pub fn container(&self) -> &HtmlElement {
		&self.container
	}

	/// Sets how long the `find_by` family waits before failing.
	pub fn with_timeout(mut self, timeout: Duration) -> Self {
		self.timeout = timeout;
		self
	}

	/// Lets leptos settle: runs pending effects so the DOM reflects every
	/// signal changed so far.
	pub async fn pump(&self) {
		leptos::task::tick().await;
	}

	/// Waits until exactly one visible element matches, then returns it, like
	/// Testing Library's `findBy`. Several matches fail at once rather than
	/// waiting for all but one to go away.
	pub async fn find_by(&self, finder: &Finder) -> Result<HtmlElement, TestError> {
		self.wait_for(finder).await?;
		self.get(finder)
	}

	/// Waits until at least one visible element matches, then returns every
	/// match, like Testing Library's `findAllBy`.
	pub async fn find_all_by(&self, finder: &Finder) -> Result<Vec<HtmlElement>, TestError> {
		self.wait_for(finder).await?;
		self.get_all(finder)
	}

	/// Waits for the element whose text is exactly `text`; see
	/// [`Self::find_by`].
	pub async fn find_by_text(&self, text: &str) -> Result<HtmlElement, TestError> {
		self.find_by(&Finder::text(text)).await
	}

	/// Waits for the element whose accessibility label is exactly `label`;
	/// see [`Self::find_by`].
	pub async fn find_by_label(&self, label: &str) -> Result<HtmlElement, TestError> {
		self.find_by(&Finder::label(label)).await
	}

	/// Waits for the element with the ARIA `role` and accessible `name`; see
	/// [`Self::find_by`].
	pub async fn find_by_role(&self, role: Role, name: &str) -> Result<HtmlElement, TestError> {
		self.find_by(&Finder::aria_role(role, name)).await
	}

	/// Waits for the element with the test identifier `id`; see
	/// [`Self::find_by`].
	pub async fn find_by_test_id(&self, id: &str) -> Result<HtmlElement, TestError> {
		self.find_by(&Finder::test_id(id)).await
	}

	/// Clicks the one enabled element `finder` matches, then pumps.
	///
	/// This is the browser's `click()`, so a button's press handler runs and
	/// a checkbox flips. A link the app does not handle navigates the test
	/// page away, as a real click would.
	pub async fn tap(&self, finder: &Finder) -> Result<(), TestError> {
		self.action_target(finder)?.click();
		self.pump().await;
		Ok(())
	}

	/// Replaces the value of the one enabled text input or text area `finder`
	/// matches, dispatches the `input` event typing would, then pumps.
	///
	/// The app owns the committed value: a handler that rejects the proposal
	/// leaves the field showing what it rendered.
	pub async fn enter_text(&self, finder: &Finder, text: &str) -> Result<(), TestError> {
		let element = self.action_target(finder)?;

		if let Some(input) = element
			.dyn_ref::<HtmlInputElement>()
			.filter(|input| accepts_text(input))
		{
			input.set_value(text);
		} else if let Some(area) = element.dyn_ref::<HtmlTextAreaElement>() {
			area.set_value(text);
		} else {
			return Err(self.error(format!("{finder:?} is not a text input")));
		}

		let init = web_sys::EventInit::new();
		init.set_bubbles(true);
		let event = web_sys::Event::new_with_event_init_dict("input", &init)
			.expect("an input event constructs");
		element
			.dispatch_event(&event)
			.expect("an element dispatches an input event");
		self.pump().await;
		Ok(())
	}

	/// Clicks the one enabled switch, checkbox, or radio `finder` matches,
	/// proposing its opposite state (a radio proposes selection), then pumps.
	pub async fn toggle(&self, finder: &Finder) -> Result<(), TestError> {
		let element = self.action_target(finder)?;
		let boolean = element
			.dyn_ref::<HtmlInputElement>()
			.is_some_and(|input| matches!(input.type_().as_str(), "checkbox" | "radio"));

		if !boolean {
			return Err(self.error(format!("{finder:?} is not a switch, checkbox, or radio")));
		}

		element.click();
		self.pump().await;
		Ok(())
	}

	/// Pumps and queries until something matches, sleeping between attempts,
	/// and fails once the timeout passes.
	async fn wait_for(&self, finder: &Finder) -> Result<(), TestError> {
		let started = now_ms();

		loop {
			self.pump().await;

			if !self.query_all(finder).is_empty() {
				return Ok(());
			}

			if now_ms() - started >= self.timeout.as_secs_f64() * 1000.0 {
				return Err(self.error(format!(
					"{finder:?} did not appear within {} ms",
					self.timeout.as_millis()
				)));
			}

			sleep(POLL_INTERVAL_MS).await;
		}
	}

	/// The one element `finder` matches, if it accepts interaction.
	fn action_target(&self, finder: &Finder) -> Result<HtmlElement, TestError> {
		let element = self.get(finder)?;

		if dom::disabled(&element) {
			return Err(self.error(format!("{finder:?} is disabled")));
		}

		Ok(element)
	}

	fn error(&self, reason: String) -> TestError {
		TestError {
			reason,
			tree: self.describe(),
		}
	}
}

/// Testing Library queries over the mounted DOM; failures show the rendered
/// markup.
///
/// Matching follows the DOM the web renderer emits, so a finder means the
/// same thing as in `leptoskit_testing::WidgetTester`:
///
/// - [`Finder::Text`]: the deepest element whose trimmed text is exactly the
///   text, so a `Text` or `Button` matches, not every ancestor.
/// - [`Finder::Label`]: `aria-label`, else the text `aria-labelledby` names.
/// - [`Finder::AriaRole`]: the `role` attribute, else the role the element
///   implies (`<button>`, `<h2>`, `<input type=checkbox>`, `<nav>`, …), with
///   the accessible name: the label, else the trimmed text.
/// - [`Finder::Role`]: the role the widget kind implies, matched as
///   [`Finder::AriaRole`]. Kinds without one, such as `View` and `Text`, match
///   nothing; find them by text, label, or test identifier.
/// - [`Finder::TestId`]: `data-testid`, which `Element::test_id` renders.
///
/// Elements that are `hidden`, `aria-hidden="true"`, `display: none`, or
/// `visibility: hidden` never match.
impl Queries for Screen {
	type Node = HtmlElement;

	fn query_all(&self, finder: &Finder) -> Vec<HtmlElement> {
		dom::visible_elements(&self.container)
			.into_iter()
			.filter(|element| dom::matches(finder, element))
			.collect()
	}

	fn describe(&self) -> String {
		dom::describe(&self.container)
	}
}

impl Drop for Screen {
	fn drop(&mut self) {
		// Dispose effects and handlers while their DOM parent still exists,
		// then remove the container so the next test gets a clean document.
		drop(self.mount.take());
		self.container.remove();
	}
}

/// Whether an `<input>` takes typed text.
fn accepts_text(input: &HtmlInputElement) -> bool {
	matches!(
		input.type_().as_str(),
		"text" | "password" | "email" | "search" | "tel" | "url" | "number"
	)
}

/// Milliseconds on the page's monotonic clock.
fn now_ms() -> f64 {
	web_sys::window()
		.and_then(|window| window.performance())
		.expect("a browser window with a performance clock")
		.now()
}

/// Resolves after `delay_ms`, letting timers, network callbacks, and the
/// browser's rendering run in between.
async fn sleep(delay_ms: i32) {
	let promise = js_sys::Promise::new(&mut |resolve, _reject| {
		web_sys::window()
			.expect("a browser window")
			.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, delay_ms)
			.expect("setTimeout accepts a function");
	});
	wasm_bindgen_futures::JsFuture::from(promise)
		.await
		.expect("a timer promise never rejects");
}
