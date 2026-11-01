//! `/overlay/highlights`: highlights what you've clipped from this page.
//! The frame stays hidden until you press a highlight, then shows that clip.

use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use leptoskit::prelude::*;

use crate::browser;
use crate::routes::Navigation;
use crate::store::Store;
use crate::ui;

static CONNECTED: AtomicBool = AtomicBool::new(false);

#[component]
pub fn Page(navigation: &Navigation) -> Element {
	let _ = navigation;
	if !browser::IN_EXTENSION {
		return view! { <ui::ExtensionOnly /> };
	}
	if !CONNECTED.swap(true, Ordering::Relaxed) {
		crate::tasks::spawn(frame::connect());
	}
	let pressed = Store::current().with(|state| state.extension.pressed.clone());
	let Some(pressed) = pressed else {
		return view! { <View /> };
	};
	view! {
		<View class="gap-2 p-3 bg-white dark:bg-stone-900">
			<Text class=ui::QUIET>{format!("From {}", pressed.project.title)}</Text>
			<Text class="italic">{format!("\u{201c}{}\u{201d}", pressed.clip.quote)}</Text>
			{match pressed.clip.note {
				Some(note) => {
					view! { <Text>{note}</Text> }
				}
				None => view! { <View /> },
			}}
			<ui::Secondary label="Close" on_press=|| crate::tasks::spawn(frame::hide()) />
		</View>
	}
}

mod frame {
	#[cfg(feature = "extension")]
	use std::cell::RefCell;

	#[cfg(feature = "extension")]
	use leptoskit::extension::page::Page;

	#[cfg(feature = "extension")]
	thread_local! {
		static PAGE: RefCell<Option<Page>> = const { RefCell::new(None) };
	}

	/// Highlights the page's clips, and shows one when it's pressed.
	pub async fn connect() {
		#[cfg(all(feature = "extension", target_arch = "wasm32"))]
		{
			use commonplace_shared::ExtensionMessage;
			use commonplace_shared::ExtensionReply;
			use leptoskit::extension::page;
			use leptoskit::extension::page::PageEvent;
			use leptoskit::extension::page::Placement;
			use leptoskit::extension::page::Quote;

			use crate::store::Store;

			let Ok(page) = page::connect().await else {
				return;
			};
			let _ = page.place(Placement::Hidden).await;
			let Ok(info) = page.info().await else { return };
			let reply = crate::browser::ask(ExtensionMessage::ClipsOnPage { url: info.url }).await;
			let ExtensionReply::Clips(clips) = reply else {
				return;
			};
			let quotes: Vec<Quote> = clips
				.iter()
				.map(|on_page| {
					Quote {
						id: on_page.clip.id.to_string(),
						exact: on_page.clip.quote.clone(),
						prefix: on_page.clip.prefix.clone(),
						suffix: on_page.clip.suffix.clone(),
					}
				})
				.collect();
			let _ = page.highlight(&quotes).await;
			Store::current().update(|state| state.extension.on_page = clips);
			let shown = page.clone();
			page.on_event(move |event| {
				if let PageEvent::HighlightPressed(id) = event {
					let store = Store::current();
					store.update(|state| {
						state.extension.pressed = state
							.extension
							.on_page
							.iter()
							.find(|on_page| on_page.clip.id.to_string() == id)
							.cloned();
					});
					let shown = shown.clone();
					crate::tasks::spawn(async move {
						let _ = shown
							.place(Placement::Corner {
								width: 340.0,
								height: 220.0,
							})
							.await;
					});
				}
			});
			PAGE.with_borrow_mut(|kept| *kept = Some(page));
		}
	}

	/// Hides the frame again, leaving the highlights.
	pub async fn hide() {
		crate::store::Store::current().update(|state| state.extension.pressed = None);
		#[cfg(all(feature = "extension", target_arch = "wasm32"))]
		{
			let page = PAGE.with_borrow(Clone::clone);
			if let Some(page) = page {
				let _ = page
					.place(leptoskit::extension::page::Placement::Hidden)
					.await;
			}
		}
	}
}
