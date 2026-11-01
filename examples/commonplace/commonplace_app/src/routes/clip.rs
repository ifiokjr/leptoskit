//! `/clip`: the extension's popup. Clip the page you're on, or hand over to
//! the clipper to pick a passage on the page itself.

use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use commonplace_shared::ExtensionMessage;
use commonplace_shared::ExtensionReply;
use commonplace_shared::NewClipInput;
use commonplace_shared::TextQuote;
use leptoskit::prelude::*;

use crate::browser;
use crate::routes::Navigation;
use crate::store::Load;
use crate::store::Store;
use crate::ui;

/// The popup reads the tab once, when it opens.
static READ_TAB: AtomicBool = AtomicBool::new(false);

thread_local! {
	static TAB: std::cell::Cell<Option<(i64, Option<i64>)>> = const { std::cell::Cell::new(None) };
}

#[component]
pub fn Page(navigation: &Navigation) -> Element {
	let _ = navigation;
	if !browser::IN_EXTENSION {
		return view! { <ui::ExtensionOnly /> };
	}
	let store = Store::current();
	if !READ_TAB.swap(true, Ordering::Relaxed) {
		let store = store.clone();
		crate::tasks::spawn(async move {
			if let Some((tab, window, page)) = browser::active_tab().await {
				TAB.set(Some((tab, window)));
				store.update(|state| state.extension.page = page);
			}
		});
	}

	let (page, projects, chosen, draft, status) = store.with(|state| {
		(
			state.extension.page.clone(),
			state.library.ready().cloned().unwrap_or_default(),
			state.extension.project,
			state.clip.clone(),
			state.extension.status.clone(),
		)
	});
	let Some(page) = page else {
		return view! {
			<View class="gap-3">
				<ui::Title text="Clip this page" />
				<Text class=ui::QUIET>"Open a web page to clip from it."</Text>
			</View>
		};
	};
	if matches!(
		store.with(|state| state.library.clone()),
		Load::Idle | Load::Loading
	) {
		return view! { <ui::Loading what="your projects" /> };
	}
	let ready = chosen.is_some() && !draft.quote.trim().is_empty();
	let save_page = page.clone();

	view! {
		<View class="gap-3">
			<ui::Title text="Clip this page" />
			{view! { <ui::PanelCard /> }
				.child(view! { <Text class="font-semibold">{page.title.clone()}</Text> })
				.child(view! { <Text class=ui::QUIET>{page.url.clone()}</Text> })}
			<ui::Primary
				label="Clip a passage on the page"
				enabled=true
				on_press=|| {
					crate::tasks::spawn(async {
						if let Some((tab, _)) = TAB.get() {
							match browser::clip_on(tab).await {
								Ok(()) => browser::close_window(),
								Err(message) => {
									Store::current()
										.update(|state| state.extension.status = Some(message))
								}
							}
						}
					});
				}
			/>
			<ui::Section text="Or type the quote" />
			<ui::ProjectPicker
				projects=&projects
				chosen=chosen
				on_pick=|project| {
					Store::current().update(|state| state.extension.project = Some(project));
					crate::tasks::spawn(browser::keep_project(project));
				}
			/>
			<ui::Field
				label="Quote"
				value=&draft.quote
				placeholder="The passage worth keeping"
				on_change=|value| {
					Store::current().update(|state| state.clip.quote = value.to_owned());
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
			<ui::Primary
				label="Save clip"
				enabled=ready
				on_press=move || {
					let store = Store::current();
					let (draft, project) =
						store.peek(|state| (state.clip.clone(), state.extension.project));
					let input = NewClipInput {
						project,
						page: save_page.clone(),
						quote: TextQuote {
							exact: draft.quote.trim().to_owned(),
							prefix: None,
							suffix: None,
						},
						note: Some(draft.note.clone()).filter(|note| !note.trim().is_empty()),
						tags: draft.tags.clone(),
					};
					crate::tasks::spawn(async move {
						let reply = browser::ask(ExtensionMessage::Clip(Box::new(input))).await;
						let store = Store::current();
						store.update(|state| {
							if matches!(reply, ExtensionReply::Saved(_) | ExtensionReply::Queued) {
								state.clip = crate::store::ClipDraft::default();
							}
							state.extension.status = Some(browser::status(&reply));
						});
					});
				}
			/>
			{match status {
				Some(status) => {
					view! { <Status class="text-sm">{status}</Status> }
				}
				None => view! { <View /> },
			}}
		</View>
	}
}
