use leptoskit::prelude::*;

#[component]
fn Greeting(name: &'static str) -> impl IntoView {
	Text(name)
}

fn main() {
	let _ = view! { <Greeting name="Ada" colour="red" /> };
}
