use leptoskit::prelude::*;

fn plain() -> Element {
	View()
}

fn main() {
	let _ = view! { <plain /> };
}
