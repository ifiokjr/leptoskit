//! The page and its islands.

use leptos::prelude::*;
use leptos_router::components::Route;
use leptos_router::components::Router;
use leptos_router::components::Routes;
use leptos_router::path;

/// The HTML document every route renders into.
pub fn shell(options: LeptosOptions) -> impl IntoView {
	view! {
		<!DOCTYPE html>
		<html lang="en">
			<head>
				<meta charset="utf-8" />
				<meta name="viewport" content="width=device-width, initial-scale=1" />
				<title>"Field notes"</title>
				<link rel="stylesheet" href="/style.css" />
				<HydrationScripts options islands=true />
			</head>
			<body>
				<App />
			</body>
		</html>
	}
}

/// The routes. Server-rendered: none of this reaches the client.
#[component]
pub fn App() -> impl IntoView {
	view! {
		<Router>
			<Routes fallback=|| "Nothing here.">
				<Route path=path!("") view=Notes />
			</Routes>
		</Router>
	}
}

/// A page of static prose with one island in it.
#[component]
fn Notes() -> impl IntoView {
	view! {
		<main class="notes">
			<p class="kicker">"Field notes"</p>
			<h1>"Most of a page is just words."</h1>
			<p>
				"This page rendered on the server. The headline, these paragraphs, and the "
				"layout around them are plain HTML: the browser shows them before any wasm "
				"arrives, and none of their code was compiled for the client."
			</p>
			<p>
				"The button below is an island. It is the only component on this page " "marked "
				<code>"#[island]"</code>", so it is the only one in the wasm "
				"bundle, and the only part of the page that hydrates."
			</p>
			<Clap start=3 />
		</main>
	}
}

/// An island: a counter whose starting value crosses from the server to the
/// client as serialized props.
#[island]
fn Clap(start: u32) -> impl IntoView {
	let (claps, set_claps) = signal(start);
	view! {
		<button class="clap" on:click=move |_| set_claps.update(|claps| *claps += 1)>
			<span aria-hidden="true">"✺"</span>
			" Clap "
			<span class="count">{claps}</span>
		</button>
	}
}
