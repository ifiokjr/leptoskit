//! The browser extension: its surfaces' entry point, and the background
//! worker.
//!
//! Every surface is one of the app's routes, rendered with an in-memory
//! router and signed in with the bearer token kept in extension storage. The
//! worker saves clips (or keeps them until the server can be reached),
//! answers the frames on other sites, and wires up the context menu, the
//! keyboard shortcut, and the toolbar badge.

use commonplace_shared::ExtensionMessage;
use commonplace_shared::ExtensionReply;
use commonplace_shared::NewClipInput;
use leptoskit::extension::Background;
use leptoskit::extension::Surface;
use leptoskit::extension::action;
use leptoskit::extension::alarms;
use leptoskit::extension::context_menus;
use leptoskit::extension::scripting;
use leptoskit::extension::storage;
use leptoskit::prelude::Element;
use leptoskit_router::Routable;

use crate::api::Api;
use crate::routes::Navigation;
use crate::routes::Route;
use crate::session;
use crate::store;
use crate::store::Store;
use crate::tasks;
use crate::transport::ExtensionFetch;

/// Clips saved while the server couldn't be reached, sent on the next alarm.
const QUEUE: &str = "commonplace.queue";
/// The alarm that sends them.
const RETRY: &str = "send-queued";
/// The project clips go to when the user hasn't picked one.
pub const DEFAULT_PROJECT: &str = "commonplace.project";

/// The server every surface talks to.
fn api() -> Api {
	let server = leptoskit::extension::runtime::server().unwrap_or(crate::SERVER);
	Api::new(ExtensionFetch {
		server: server.to_owned(),
	})
}

/// Opens a surface at its configured route.
pub fn main(surface: Surface) -> impl Fn() -> Element + Send + Sync + 'static {
	store::install(api());
	let route = Route::parse(surface.route()).unwrap_or(Route::Library);
	tasks::spawn(async {
		let token = session::kept().await;
		let project = storage::LOCAL
			.get::<commonplace_shared::ProjectId>(DEFAULT_PROJECT)
			.await
			.ok()
			.flatten();
		let store = Store::current();
		store.update(|state| state.extension.project = project);
		store.start(token);
	});
	let navigation = Navigation::new(route);
	move || navigation.render()
}

/// Registers the worker's handlers.
pub fn background(worker: &Background) {
	store::install(api());

	worker.on_installed(|_| {
		async {
			let _ = context_menus::remove_all().await;
			let _ = context_menus::create(
				"clip",
				"Clip \u{201c}%s\u{201d} to Commonplace",
				&[context_menus::Context::Selection],
			)
			.await;
			let _ = alarms::every(RETRY, 1.0).await;
			let _ = action::set_badge_color("#b45309").await;
		}
	});

	// The menu item and the shortcut both open the clipper on the page; the
	// click is what lets the extension reach that page (`activeTab`).
	worker.on_context_menu(|click| {
		async move {
			if let Some(tab) = click.tab.and_then(|tab| tab.id) {
				let _ = scripting::inject(tab, "clipper").await;
			}
		}
	});
	worker.on_command(|command| {
		async move {
			if command.command == "clip"
				&& let Some(tab) = command.tab.and_then(|tab| tab.id)
			{
				let _ = scripting::inject(tab, "clipper").await;
			}
		}
	});

	worker.on_alarm(|alarm| {
		async move {
			if alarm.name == RETRY {
				send_queued().await;
			}
		}
	});

	worker.on_tab_updated(|update| {
		async move {
			let complete = update.change_info["status"].as_str() == Some("complete");
			if let (true, Some(url)) = (complete, update.tab.url) {
				badge(update.tab_id, &url).await;
			}
		}
	});

	worker.on_message(|message: ExtensionMessage, sender| {
		async move {
			match message {
				ExtensionMessage::Clip(input) => {
					let reply = save(*input).await;
					if let (ExtensionReply::Saved(_), Some(tab)) = (&reply, sender.tab)
						&& let (Some(id), Some(url)) = (tab.id, tab.url)
					{
						badge(id, &url).await;
					}
					reply
				}
				ExtensionMessage::ClipsOnPage { url } => {
					match signed_in_api().await.clips_on_page(&url).await {
						Ok(clips) => ExtensionReply::Clips(clips),
						Err(problem) => ExtensionReply::Failed(problem.message),
					}
				}
				ExtensionMessage::SessionChanged => ExtensionReply::Done,
			}
		}
	});
}

/// The API client with the token the surfaces signed in with: read each
/// time, since a surface may have signed in or out since.
async fn signed_in_api() -> Api {
	let api = store::api();
	api.set_token(session::kept().await);
	api
}

/// Saves a clip, or keeps it for later when the server can't be reached.
async fn save(input: NewClipInput) -> ExtensionReply {
	match signed_in_api().await.create_clip(&input).await {
		Ok(clip) => ExtensionReply::Saved(clip),
		Err(problem) if problem.unreachable => {
			let mut queue: Vec<NewClipInput> = storage::LOCAL
				.get(QUEUE)
				.await
				.ok()
				.flatten()
				.unwrap_or_default();
			queue.push(input);
			match storage::LOCAL.set(QUEUE, &queue).await {
				Ok(()) => ExtensionReply::Queued,
				Err(error) => ExtensionReply::Failed(error.to_string()),
			}
		}
		Err(problem) if problem.signed_out => {
			ExtensionReply::Failed(
				"Sign in to Commonplace in the extension's options first.".into(),
			)
		}
		Err(problem) => ExtensionReply::Failed(problem.message),
	}
}

/// Sends the clips kept while offline, keeping any that still can't go.
async fn send_queued() {
	let queue: Vec<NewClipInput> = storage::LOCAL
		.get(QUEUE)
		.await
		.ok()
		.flatten()
		.unwrap_or_default();
	if queue.is_empty() {
		return;
	}
	let api = signed_in_api().await;
	let mut kept = Vec::new();
	for input in queue {
		match api.create_clip(&input).await {
			Ok(_) => {}
			// The server refused it: keeping it would retry forever.
			Err(problem) if !problem.unreachable => {}
			Err(_) => kept.push(input),
		}
	}
	let _ = if kept.is_empty() {
		storage::LOCAL.remove(QUEUE).await
	} else {
		storage::LOCAL.set(QUEUE, &kept).await
	};
}

/// Shows how many clips came from the page on the toolbar button.
async fn badge(tab: i64, url: &str) {
	let count = signed_in_api()
		.await
		.clips_on_page(url)
		.await
		.map_or(0, |clips| clips.len());
	let text = if count == 0 {
		String::new()
	} else {
		count.to_string()
	};
	let _ = action::set_badge(&text, Some(tab)).await;
}
