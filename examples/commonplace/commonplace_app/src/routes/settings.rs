//! `/settings`: the extension's options. The account, the project clips go
//! to, and the sites that show your highlights.

use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use leptoskit::prelude::*;

use crate::browser;
use crate::routes::Navigation;
use crate::store::Store;
use crate::ui;

static READ_SITES: AtomicBool = AtomicBool::new(false);

pub fn page(navigation: &Navigation) -> Element {
	let _ = navigation;
	if !browser::IN_EXTENSION {
		return ui::extension_only();
	}
	let store = Store::current();
	if !READ_SITES.swap(true, Ordering::Relaxed) {
		crate::tasks::spawn(async {
			let sites = highlights::registered().await;
			Store::current().update(|state| state.extension.sites = sites);
		});
	}
	let (projects, chosen, sites, site, status) = store.with(|state| {
		(
			state.library.ready().cloned().unwrap_or_default(),
			state.extension.project,
			state.extension.sites.clone(),
			state.extension.site.clone(),
			state.extension.status.clone(),
		)
	});

	View()
		.class("gap-4")
		.child(ui::title("Commonplace settings"))
		.child(
			ui::card()
				.child(ui::section("Clip into"))
				.child(Text("The project the clipper picks first.").class(ui::QUIET))
				.child(ui::project_picker(&projects, chosen, |project| {
					Store::current().update(|state| state.extension.project = Some(project));
					crate::tasks::spawn(browser::keep_project(project));
				})),
		)
		.child(
			ui::card()
				.child(ui::section("Highlights"))
				.child(
					Text(
						"Commonplace highlights what you've clipped on the sites you choose. Each \
						 site asks for your permission once.",
					)
					.class(ui::QUIET),
				)
				.children(sites.iter().map(|pattern| {
					let remove = pattern.clone();
					View()
						.class("flex-row items-center justify-between gap-2")
						.child(Text(pattern.clone()))
						.child(ui::secondary(
							&format!("Stop highlighting {pattern}"),
							move || {
								let pattern = remove.clone();
								crate::tasks::spawn(async move {
									let sites = highlights::remove(&pattern).await;
									Store::current().update(|state| state.extension.sites = sites);
								});
							},
						))
				}))
				.child(ui::field(
					"Site address",
					&site,
					"https://www.theguardian.com",
					|value| {
						Store::current().update(|state| state.extension.site = value.to_owned());
					},
				))
				.child(ui::primary(
					"Highlight on this site",
					!site.trim().is_empty(),
					|| {
						let site = Store::current().peek(|state| state.extension.site.clone());
						crate::tasks::spawn(async move {
							let result = highlights::add(&site).await;
							Store::current().update(|state| {
								match result {
									Ok(sites) => {
										state.extension.sites = sites;
										state.extension.site.clear();
										state.extension.status = None;
									}
									Err(message) => state.extension.status = Some(message),
								}
							});
						});
					},
				))
				.child(match status {
					Some(status) => Status(status).class("text-sm"),
					None => View(),
				}),
		)
}

/// The `highlights` content script's sites: asking for each one's
/// permission, and registering the script for them.
mod highlights {
	#[cfg(feature = "extension")]
	use leptoskit::extension::permissions;
	#[cfg(feature = "extension")]
	use leptoskit::extension::scripting;

	const SCRIPT: &str = "highlights";

	pub async fn registered() -> Vec<String> {
		#[cfg(feature = "extension")]
		{
			return scripting::registered(SCRIPT)
				.await
				.ok()
				.flatten()
				.unwrap_or_default();
		}
		#[allow(unreachable_code)]
		Vec::new()
	}

	pub async fn add(site: &str) -> Result<Vec<String>, String> {
		#[cfg(feature = "extension")]
		{
			let pattern = permissions::site_pattern(site.trim())
				.ok_or("Enter a site's address, starting with https://")?;
			let granted = permissions::request(std::slice::from_ref(&pattern))
				.await
				.map_err(|error| error.to_string())?;
			if !granted {
				return Err("Commonplace needs your permission to highlight on that site.".into());
			}
			let mut sites = registered().await;
			if !sites.contains(&pattern) {
				sites.push(pattern);
			}
			register(&sites).await?;
			return Ok(sites);
		}
		#[allow(unreachable_code)]
		{
			let _ = (site, SCRIPT);
			Err("This only works in the browser extension.".into())
		}
	}

	pub async fn remove(pattern: &str) -> Vec<String> {
		let mut sites = registered().await;
		sites.retain(|site| site != pattern);
		#[cfg(feature = "extension")]
		{
			let _ = register(&sites).await;
			let _ = permissions::remove(&[pattern.to_owned()]).await;
		}
		sites
	}

	#[cfg(feature = "extension")]
	async fn register(sites: &[String]) -> Result<(), String> {
		let _ = scripting::unregister(SCRIPT).await;
		if sites.is_empty() {
			return Ok(());
		}
		scripting::register(SCRIPT, sites)
			.await
			.map_err(|error| error.to_string())
	}
}
