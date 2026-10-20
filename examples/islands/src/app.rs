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
				<Route path=path!("lab") view=Lab />
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
			<p>
				"The "<a href="/lab">"colour lab"</a>" has a heavier island, marked "
				<code>"#[island(lazy)]"</code>". Its code is split into its own wasm chunk, so "
				"this page never downloads it."
			</p>
		</main>
	}
}

/// A page whose only island is lazy: its chunk loads here, and nowhere else.
#[component]
fn Lab() -> impl IntoView {
	view! {
		<main class="notes">
			<p class="kicker">"Colour lab"</p>
			<h1>"This island loaded on demand."</h1>
			<p>
				"The mixer below lives in a wasm chunk of its own. This page fetched it as "
				"the island hydrated; the notes page never does."
			</p>
			<Mixer />
			<p>
				<a href="/">"← Back to the notes"</a>
			</p>
		</main>
	}
}

/// A lazy island: an OKLCH colour mixer. `lazy` moves it, and the colour
/// maths only it uses, into a wasm chunk that loads the first time a page
/// renders it.
#[island(lazy)]
fn Mixer() -> impl IntoView {
	let (lightness, set_lightness) = signal(0.68);
	let (chroma, set_chroma) = signal(0.17);
	let (hue, set_hue) = signal(28.0);
	let colour = move || oklch_to_hex(lightness.get(), chroma.get(), hue.get());
	let slider = move |label: &'static str,
	                   max: &'static str,
	                   step: &'static str,
	                   value: ReadSignal<f64>,
	                   set: WriteSignal<f64>| {
		view! {
			<label>
				<span>{label}</span>
				<input
					type="range"
					min="0"
					max=max
					step=step
					prop:value=move || value.get().to_string()
					on:input=move |event| {
						if let Ok(next) = event_target_value(&event).parse() {
							set.set(next);
						}
					}
				/>
			</label>
		}
	};
	view! {
		<div class="mixer">
			<div class="swatch" style:background=colour>
				<code>{colour}</code>
			</div>
			{slider("Lightness", "1", "0.01", lightness, set_lightness)}
			{slider("Chroma", "0.37", "0.005", chroma, set_chroma)}
			{slider("Hue", "360", "1", hue, set_hue)}
		</div>
	}
}

/// An OKLCH colour as a `#rrggbb` sRGB hex string, clamped into gamut.
fn oklch_to_hex(lightness: f64, chroma: f64, hue_degrees: f64) -> String {
	let hue = hue_degrees.to_radians();
	// OKLab's opponent axes, then the long, medium, and short cone responses.
	let (green_red, blue_yellow) = (chroma * hue.cos(), chroma * hue.sin());
	let long = (lightness + 0.396_337_777_4 * green_red + 0.215_803_757_3 * blue_yellow).powi(3);
	let medium = (lightness - 0.105_561_345_8 * green_red - 0.063_854_172_8 * blue_yellow).powi(3);
	let short = (lightness - 0.089_484_177_5 * green_red - 1.291_485_548_0 * blue_yellow).powi(3);
	let linear = [
		4.076_741_662_1 * long - 3.307_711_591_3 * medium + 0.230_969_929_2 * short,
		-1.268_438_004_6 * long + 2.609_757_401_1 * medium - 0.341_319_396_5 * short,
		-0.004_196_086_3 * long - 0.703_418_614_7 * medium + 1.707_614_701_0 * short,
	];
	let [red, green, blue] = linear.map(|channel| {
		let encoded = if channel <= 0.003_130_8 {
			12.92 * channel
		} else {
			1.055 * channel.powf(1.0 / 2.4) - 0.055
		};
		(encoded.clamp(0.0, 1.0) * 255.0).round() as u8
	});
	format!("#{red:02x}{green:02x}{blue:02x}")
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
