//! `/sign-in`. The `(auth)` group adds nothing to the URL.

use leptoskit::prelude::*;

/// The sign-in form.
#[component]
pub fn Page() -> Element {
	view! { <Button class="m-6 px-4 py-2 rounded-lg">"Sign in"</Button> }
}
