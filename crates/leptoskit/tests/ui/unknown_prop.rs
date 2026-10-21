use leptoskit::prelude::*;

#[component]
fn Greeting(name: String) -> impl IntoView {
	Text(name)
}

fn main() {
	let _ = view! { <Greeting name="Ada".to_owned() colour="red" /> };
}
