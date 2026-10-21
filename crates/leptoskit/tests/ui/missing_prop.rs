use leptoskit::prelude::*;

#[component]
fn Greeting(name: String, #[prop(optional)] excited: bool) -> impl IntoView {
	Text(if excited {
		format!("Hi {name}!")
	} else {
		format!("Hi {name}")
	})
}

fn main() {
	let _ = view! { <Greeting excited /> };
}
