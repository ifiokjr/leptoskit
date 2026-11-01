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

#[component]
pub fn Page(navigation: &Navigation) -> Element {
	let _ = navigation;
	if !browser::IN_EXTENSION {
		return view! { <ui::ExtensionOnly /> };
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

	view! {
		<View class="gap-4">
			<ui::Title text="Commonplace settings" />
			{view! { <ui::PanelCard /> }
				.child(view! { <ui::Section text="Clip into" /> })
				.child(view! {
					<Text class=ui::QUIET>"The project the clipper picks first."</Text>
				})
				.child(view! {
					<ui::ProjectPicker
						projects=&projects
						chosen=chosen
						on_pick=|project| {
							Store::current()
								.update(|state| state.extension.project = Some(project));
							crate::tasks::spawn(browser::keep_project(project));
						}
					/>
				})}
			{view! { <ui::PanelCard /> }
				.child(view! { <ui::Section text="Highlights" /> })
				.child(view! {
					<Text class=ui::QUIET>
						"Commonplace highlights what you've clipped on the sites you choose. Each \
						 site asks for your permission once."
					</Text>
				})
				.children(sites.iter().map(|pattern| {
					let remove = pattern.clone();
					view! {
						<View class="flex-row items-center justify-between gap-2">
							<Text>{pattern.clone()}</Text>
							<ui::Secondary
								label=&format!("Stop highlighting {pattern}")
								on_press=move || {
									let pattern = remove.clone();
									crate::tasks::spawn(async move {
										let sites = highlights::remove(&pattern).await;
										Store::current()
											.update(|state| state.extension.sites = sites);
									});
								}
							/>
						</View>
					}
				}))
				.child(view! {
					<ui::Field
						label="Site address"
						value=&site
						placeholder="https://www.theguardian.com"
						on_change=|value| {
							Store::current()
								.update(|state| state.extension.site = value.to_owned());
						}
					/>
				})
				.child(view! {
					<ui::Primary
						label="Highlight on this site"
						enabled=!site.trim().is_empty()
						on_press=|| {
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
						}
					/>
				})
				.child(match status {
					Some(status) => {
						view! { <Status class="text-sm">{status}</Status> }
					}
					None => view! { <View /> },
				})}
		</View>
	}
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
