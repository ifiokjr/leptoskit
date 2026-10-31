//! What the extension's screens ask of the browser, in one place, so the
//! routes stay free of `cfg`: outside the extension each call does nothing.

use commonplace_shared::ExtensionMessage;
use commonplace_shared::ExtensionReply;
use commonplace_shared::PageInput;

/// Whether these screens are running in the browser extension.
pub const IN_EXTENSION: bool = cfg!(feature = "extension");

/// Asks the extension's worker, and waits for its answer.
pub async fn ask(message: ExtensionMessage) -> ExtensionReply {
	#[cfg(feature = "extension")]
	{
		return leptoskit::extension::runtime::send(&message)
			.await
			.unwrap_or_else(|error| ExtensionReply::Failed(error.to_string()));
	}
	#[allow(unreachable_code)]
	{
		let _ = message;
		ExtensionReply::Failed("This only works in the browser extension.".into())
	}
}

/// The tab the user is looking at: its ID, window, and page, when the
/// extension may read it.
pub async fn active_tab() -> Option<(i64, Option<i64>, Option<PageInput>)> {
	#[cfg(feature = "extension")]
	{
		let tab = leptoskit::extension::tabs::active().await.ok()??;
		let page = tab
			.url
			.filter(|url| url.starts_with("https://") || url.starts_with("http://"))
			.map(|url| {
				PageInput {
					title: tab.title.clone().unwrap_or_else(|| url.clone()),
					url,
					..PageInput::default()
				}
			});
		return Some((tab.id?, tab.window_id, page));
	}
	#[allow(unreachable_code)]
	None
}

/// Shows the clipper on a tab.
pub async fn clip_on(tab: i64) -> Result<(), String> {
	#[cfg(feature = "extension")]
	{
		return leptoskit::extension::scripting::inject(tab, "clipper")
			.await
			.map_err(|error| error.to_string());
	}
	#[allow(unreachable_code)]
	{
		let _ = tab;
		Err("This only works in the browser extension.".into())
	}
}

/// Opens an address in a new tab.
pub async fn open(url: &str) {
	#[cfg(feature = "extension")]
	{
		let _ = leptoskit::extension::tabs::open(url).await;
	}
	#[cfg(not(feature = "extension"))]
	let _ = url;
}

/// Closes this window: the popup, once it has handed over to the page.
pub fn close_window() {
	#[cfg(all(feature = "extension", target_arch = "wasm32"))]
	{
		let global = js_sys::global();
		if let Ok(close) = js_sys::Reflect::get(&global, &"close".into())
			&& let Ok(close) = wasm_bindgen::JsCast::dyn_into::<js_sys::Function>(close)
		{
			let _ = close.call0(&global);
		}
	}
}

/// Remembers the project clips go to.
pub async fn keep_project(project: commonplace_shared::ProjectId) {
	#[cfg(feature = "extension")]
	{
		let _ = leptoskit::extension::storage::LOCAL
			.set(crate::extension::DEFAULT_PROJECT, &project)
			.await;
	}
	#[cfg(not(feature = "extension"))]
	let _ = project;
}

/// What a saved, queued, or failed clip says to the user.
#[must_use]
pub fn status(reply: &ExtensionReply) -> String {
	match reply {
		ExtensionReply::Saved(_) => "Saved to your project.".into(),
		ExtensionReply::Queued => {
			"You're offline, so it's kept here and sent when Commonplace is reachable.".into()
		}
		ExtensionReply::Failed(message) => message.clone(),
		ExtensionReply::Clips(_) | ExtensionReply::Done => String::new(),
	}
}
