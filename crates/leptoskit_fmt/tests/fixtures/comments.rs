use leptos::prelude::*;

#[component]
pub fn Commented() -> impl IntoView {
	view! {
		// Leading comment for the whole tree.
		<main>
			// The heading.
			<h1>"Title"</h1> // trailing after the heading

			/* A block comment on its own line. */
			<p
				// Why this class:
				class="lead" // trailing an attribute
				id="intro"
			>"Intro"</p>
			<div>
				// Only a comment inside.
			</div>
			<span>"a"</span> /* inline block */ <span>"b"</span>
			{
				// A comment inside a block stays with rustfmt.
				let greeting = "hi";
				greeting
			}
			// Dangling at the end of main.
		</main>
		// Dangling at the end of the macro.
	}
}
