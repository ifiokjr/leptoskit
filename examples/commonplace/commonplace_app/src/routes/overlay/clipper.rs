//! `/overlay/clipper`: the clipper, in a frame on the page you're reading.
//! It takes the selection with a little text on each side, so the quote can
//! be found and highlighted on the page again.

use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use commonplace_shared::ExtensionMessage;
use commonplace_shared::ExtensionReply;
use commonplace_shared::NewClipInput;
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
	let store = Store::current();
	let (quote, page, projects, chosen, draft, status) = store.with(|state| {
		(
			state.extension.quote.clone(),
			state.extension.page.clone(),
			state.library.ready().cloned().unwrap_or_default(),
			state.extension.project,
			state.clip.clone(),
			state.extension.status.clone(),
		)
	});
	let close =
		view! { <ui::Secondary label="Close" on_press=|| crate::tasks::spawn(frame::close()) /> };
	let Some(quote) = quote else {
		return view! {
			<View class="gap-2 p-3">
				<ui::Section text="Clip to Commonplace" />
				<Text class=ui::QUIET>"Select the passage you want, and it appears here."</Text>
				{close}
			</View>
		};
	};
	let ready = chosen.is_some() && page.is_some();
	view! {
		<View class="gap-2 p-3 bg-white dark:bg-stone-900">
			<ui::Section text="Clip to Commonplace" />
			<Text class="italic">{format!("\u{201c}{}\u{201d}", quote.exact)}</Text>
			<ui::ProjectPicker
				projects=&projects
				chosen=chosen
				on_pick=|project| {
					Store::current().update(|state| state.extension.project = Some(project));
					crate::tasks::spawn(browser::keep_project(project));
				}
			/>
			<ui::Field
				label="Your note"
				value=&draft.note
				placeholder="Optional"
				on_change=|value| {
					Store::current().update(|state| state.clip.note = value.to_owned());
				}
			/>
			<ui::Field
				label="Tags"
				value=&draft.tags
				placeholder="method, cities"
				on_change=|value| {
					Store::current().update(|state| state.clip.tags = value.to_owned());
				}
			/>
			<View class="flex-row gap-2">
				<ui::Primary
					label="Save clip"
					enabled=ready
					on_press=move || {
						let store = Store::current();
						let input = store.peek(|state| {
							NewClipInput {
								project: state.extension.project,
								page: state.extension.page.clone().unwrap_or_default(),
								quote: state.extension.quote.clone().unwrap_or_default(),
								note: Some(state.clip.note.clone())
									.filter(|note| !note.trim().is_empty()),
								tags: state.clip.tags.clone(),
							}
						});
						crate::tasks::spawn(async move {
							let reply = browser::ask(ExtensionMessage::Clip(Box::new(input))).await;
							Store::current().update(|state| {
								if matches!(
									reply,
									ExtensionReply::Saved(_) | ExtensionReply::Queued
								) {
									state.clip = crate::store::ClipDraft::default();
								}
								state.extension.status = Some(browser::status(&reply));
							});
						});
					}
				/>
				{close}
			</View>
			{match status {
				Some(status) => {
					view! { <Status class="text-sm">{status}</Status> }
				}
				None => view! { <View /> },
			}}
		</View>
	}
}

/// The page this frame sits on.
mod frame {
	#[cfg(feature = "extension")]
	use std::cell::RefCell;

	#[cfg(feature = "extension")]
	use leptoskit::extension::page::Page;

	#[cfg(feature = "extension")]
	thread_local! {
		static PAGE: RefCell<Option<Page>> = const { RefCell::new(None) };
	}

	/// Reads the selection and the page, follows the selection as it
	/// changes, and places the frame beside it.
	pub async fn connect() {
		#[cfg(all(feature = "extension", target_arch = "wasm32"))]
		{
			use commonplace_shared::PageInput;
			use commonplace_shared::TextQuote;
			use leptoskit::extension::page;
			use leptoskit::extension::page::PageEvent;
			use leptoskit::extension::page::Placement;

			use crate::store::Store;

			let Ok(page) = page::connect().await else {
				return;
			};
			let info = page.info().await.unwrap_or_default();
			let quote = |selection: page::Selection| {
				TextQuote {
					exact: selection.text,
					prefix: Some(selection.before).filter(|text| !text.is_empty()),
					suffix: Some(selection.after).filter(|text| !text.is_empty()),
				}
			};
			let selection = page.selection().await.ok().flatten();
			let found = selection.is_some();
			Store::current().update(|state| {
				state.extension.page = Some(PageInput {
					url: info.url,
					title: info.title,
					author: info.author,
					site: info.site,
					published: info.published,
				});
				state.extension.quote = selection.map(quote);
			});
			page.on_event(move |event| {
				if let PageEvent::Selection(Some(selection)) = event {
					Store::current().update(|state| {
						state.extension.quote = Some(quote(selection));
						state.extension.status = None;
					});
				}
			});
			let _ = page
				.place(if found {
					Placement::NearSelection {
						width: 380.0,
						height: 360.0,
					}
				} else {
					Placement::Corner {
						width: 380.0,
						height: 160.0,
					}
				})
				.await;
			PAGE.with_borrow_mut(|kept| *kept = Some(page));
		}
	}

	/// Takes the frame off the page.
	pub async fn close() {
		#[cfg(feature = "extension")]
		{
			let page = PAGE.with_borrow_mut(Option::take);
			if let Some(page) = page {
				let _ = page.close().await;
			}
		}
	}
}
