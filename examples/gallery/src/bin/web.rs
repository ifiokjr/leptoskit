//! The same gallery screens mounted in the browser with Leptos reactivity.

#[cfg(target_arch = "wasm32")]
fn main() {
	use gallery::Assets;
	use gallery::SCREENS;
	use gallery::ScreenState;
	use leptos::prelude::*;
	use leptoskit::prelude::Button;
	use leptoskit::prelude::Text;
	use leptoskit::prelude::View;
	use leptoskit::yoga::Dimension;

	let assets = Assets::for_browser().expect("the gallery artwork fits the BMP format");
	mount_to_body(move || {
		let revision = RwSignal::new(0_u64);
		let selected = RwSignal::new(0_usize);
		let state =
			ScreenState::new(assets).on_change(move || revision.update(|value| *value += 1));
		move || {
			let _ = revision.get();
			let selected_index = selected.get();
			let screen = &SCREENS[selected_index];
			let picker = View()
				.class("flex-row flex-wrap justify-center gap-2")
				.children(SCREENS.iter().enumerate().map(|(index, screen)| {
					Button(screen.title)
						.class("px-4 py-2 rounded-[20px] text-[14px] font-semibold")
						.class(if index == selected_index {
							"bg-[#16111f] text-[#ffffff]"
						} else {
							"bg-[#ffffff] text-[#16111f]"
						})
						.on_press(move || selected.set(index))
				}));
			let preview = View()
				.with_style(|style| {
					style.width = Dimension::Points(screen.viewport.0);
					style.height = Dimension::Points(screen.viewport.1);
				})
				.child(screen.render(&state));
			leptoskit::web::render(
				View()
					.class("min-h-screen bg-[#e8e7ec] p-3 md:p-6 gap-6 flex-col items-center")
					.child(Text("leptoskit gallery").class("text-[24px] font-bold text-[#16111f]"))
					.child(picker)
					.child(preview),
			)
		}
	});
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
	eprintln!("Run `cd examples/gallery && trunk serve` to open the web gallery.");
}
