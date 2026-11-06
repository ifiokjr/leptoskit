//! The page: buttons that send events, and a summary that updates live.

use std::collections::BTreeMap;

use analytics_service::Capture;
use analytics_service::EventSummary;
use analytics_service::Summary;
use leptos::prelude::*;
use leptos_router::components::Route;
use leptos_router::components::Router;
use leptos_router::components::Routes;
use leptos_router::path;
use leptoskit_query::QueryClient;
use leptoskit_query::QueryHandle;
use leptoskit_rpc::FnError;
use leptoskit_rpc::client::call;
use leptoskit_rpc::use_live;

/// How many days the summary covers.
pub const DAYS: u32 = 7;

/// The HTML document every route renders into.
pub fn shell(options: LeptosOptions) -> impl IntoView {
	view! {
		<!DOCTYPE html>
		<html lang="en">
			<head>
				<meta charset="utf-8" />
				<meta name="viewport" content="width=device-width, initial-scale=1" />
				<title>"Analytics"</title>
				<link rel="stylesheet" href="/style.css" />
				<HydrationScripts options />
			</head>
			<body>
				<App />
			</body>
		</html>
	}
}

/// The routes, with the query cache the summary lives in.
#[component]
pub fn App() -> impl IntoView {
	QueryClient::new().provide();

	view! {
		<Router>
			<Routes fallback=|| "Nothing here.">
				<Route path=path!("") view=Dashboard />
			</Routes>
		</Router>
	}
}

/// Two buttons that send events, and what's been sent this week.
#[component]
fn Dashboard() -> impl IntoView {
	// The service runs `summary`; this watches it over the service's live
	// connection, and the server renders its first value.
	let summary = use_live(|| Summary { days: DAYS });
	let problem = RwSignal::new(None::<String>);
	let send = move |event: &'static str| {
		leptos::task::spawn_local(async move {
			let sent = call(Capture {
				event: event.to_owned(),
				distinct_id: visitor_id(),
				properties: BTreeMap::from([("page".to_owned(), "/".to_owned())]),
			})
			.await;
			problem.set(sent.err().map(|error| error.to_string()));
		});
	};

	view! {
		<main>
			<p class="kicker">"Analytics service"</p>
			<h1>"Events, counted somewhere else."</h1>
			<p>
				"This page rendered on the app's server. The counts below come from the "
				"analytics service, a separate process with its own database. The buttons "
				"send it events, and the counts update as they arrive."
			</p>
			<div class="buttons">
				<button on:click=move |_| send("clicked_buy")>"Buy"</button>
				<button on:click=move |_| send("clicked_share")>"Share"</button>
			</div>
			{move || {
				problem
					.get()
					.map(|problem| view! { <p class="problem">{problem}</p> })
			}}
			<Suspense fallback=|| view! { <p>"Counting…"</p> }>
				{move || {
					Suspend::new(async move {
						match summary.await {
							Ok(_) => view! { <Counts summary /> }.into_any(),
							Err(error) => {
								view! { <p class="problem">{error.to_string()}</p> }.into_any()
							}
						}
					})
				}}
			</Suspense>
		</main>
	}
}

/// The summary's totals and its events, read reactively so live updates
/// show without reloading.
#[component]
fn Counts(summary: QueryHandle<EventSummary, FnError>) -> impl IntoView {
	let current = move || summary.data().unwrap_or_default();

	view! {
		<div class="totals">
			<p>"Events: " <strong>{move || current().total}</strong></p>
			<p>"Visitors: " <strong>{move || current().visitors}</strong></p>
		</div>
		<table>
			<thead>
				<tr>
					<th>"Event"</th>
					<th>"Count"</th>
				</tr>
			</thead>
			<tbody>
				<For each=move || current().events key=|event| event.name.clone() let(event)>
					<tr>
						<td>{event.name}</td>
						<td>{event.count}</td>
					</tr>
				</For>
			</tbody>
		</table>
	}
}

/// This browser's visitor id: made once and kept in `localStorage`.
#[cfg(feature = "hydrate")]
fn visitor_id() -> String {
	const KEY: &str = "analytics_visitor";
	let storage = web_sys::window().and_then(|window| window.local_storage().ok().flatten());

	if let Some(id) = storage
		.as_ref()
		.and_then(|storage| storage.get_item(KEY).ok().flatten())
	{
		return id;
	}

	let id = format!(
		"visitor-{:016x}",
		(js_sys::Math::random() * 2f64.powi(53)) as u64
	);

	if let Some(storage) = storage {
		let _ = storage.set_item(KEY, &id);
	}

	id
}

/// Off the browser, buttons never run.
#[cfg(not(feature = "hydrate"))]
fn visitor_id() -> String {
	"server".to_owned()
}
