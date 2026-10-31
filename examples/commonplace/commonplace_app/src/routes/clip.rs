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

pub fn page(navigation: &Navigation) -> Element {
	let _ = navigation;
	if !browser::IN_EXTENSION {
		return ui::extension_only();
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
		return View()
			.class("gap-3")
			.child(ui::title("Clip this page"))
			.child(Text("Open a web page to clip from it.").class(ui::QUIET));
	};
	if matches!(
		store.with(|state| state.library.clone()),
		Load::Idle | Load::Loading
	) {
		return ui::loading("your projects");
	}
	let ready = chosen.is_some() && !draft.quote.trim().is_empty();
	let save_page = page.clone();

	View()
		.class("gap-3")
		.child(ui::title("Clip this page"))
		.child(
			ui::card()
				.child(Text(page.title.clone()).class("font-semibold"))
				.child(Text(page.url.clone()).class(ui::QUIET)),
		)
		.child(ui::primary("Clip a passage on the page", true, || {
			crate::tasks::spawn(async {
				if let Some((tab, _)) = TAB.get() {
					match browser::clip_on(tab).await {
						Ok(()) => browser::close_window(),
						Err(message) => {
							Store::current().update(|state| state.extension.status = Some(message))
						}
					}
				}
			});
		}))
		.child(ui::section("Or type the quote"))
		.child(ui::project_picker(&projects, chosen, |project| {
			Store::current().update(|state| state.extension.project = Some(project));
			crate::tasks::spawn(browser::keep_project(project));
		}))
		.child(ui::field(
			"Quote",
			&draft.quote,
			"The passage worth keeping",
			|value| {
				Store::current().update(|state| state.clip.quote = value.to_owned());
			},
		))
		.child(ui::field("Your note", &draft.note, "Optional", |value| {
			Store::current().update(|state| state.clip.note = value.to_owned());
		}))
		.child(ui::field("Tags", &draft.tags, "method, cities", |value| {
			Store::current().update(|state| state.clip.tags = value.to_owned());
		}))
		.child(ui::primary("Save clip", ready, move || {
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
		}))
		.child(match status {
			Some(status) => Status(status).class("text-sm"),
			None => View(),
		})
}
