use leptos::prelude::*;

#[component]
pub fn Broken() -> impl IntoView {
	let fine = view! {   <p>"formatted"</p>   };
	view! {
		<div>
			<p>"unclosed"
		</div>
	}
}
