use leptoskit::prelude::*;

fn main() {
	let _ = view! {
		<View data-id="x" on:press(event)>
			<Text>"unclosed"
		</View>
	};
}
