# Debugging with DevTools

A button that looks right in the browser is two pixels taller on iOS, and the Android emulator shows an old list. On the web you would open the browser's developer tools, but most of a leptoskit app doesn't run in a browser: on iOS, Android, and the desktop, your interface runs inside the app and draws real native widgets, so there is no page to inspect.

leptoskit DevTools fills that gap. Every app that `leptoskit dev` runs reports to one local hub, and one page shows them all: the element tree on each platform, where each element was built in your code, its frame and its Tailwind classes, and your app's route, data, and timings. Select an element on the web and see the same element's frame on iOS beside it.

## Opening it

`leptoskit dev` starts the DevTools hub next to your app and prints its address:

```text
[devtools] hub at http://127.0.0.1:7340
[web] serving dist at http://127.0.0.1:3000
```

Open the hub's address in any browser. It needs no extension and no account, which also makes it the way to inspect an app running in the iOS Simulator or on an Android emulator.

| Flag                     | What it does                            |
| ------------------------ | --------------------------------------- |
| `--devtools-port <port>` | Runs the hub on another port than 7340. |
| `--no-devtools`          | Doesn't start the hub.                  |

The hub listens only on this machine, and refuses requests from other websites open in your browser, so a page you visit can't read your app's state or send it commands.

## In the browser's developer tools

When you work on the web, the browser's developer tools are already open beside your page. The DevTools panel puts this page there too, as a **leptoskit** tab next to Elements and Console, in Chrome, Edge, and Firefox. It shows the session of the tab you're inspecting, follows it when you reload, and still lists your app's other platforms, so you can switch to the iOS Simulator from the same tab. Safari has no developer tools API for extensions, so there is no panel for it.

The panel doesn't carry its own copy of DevTools. It shows the page of the hub that `leptoskit dev` runs, so it always matches your leptoskit version. When no hub is running, it says how to start one. If you run the hub with `--devtools-port`, enter that port in the panel's bar; the panel remembers it.

The panel isn't in the browsers' extension stores yet. Until it is, open the hub's address in a tab.

## What it shows

The left column lists every running app, grouped by name, with its platform and device: `web`, `iOS · iPhone 17`, `Android · Pixel 9`, `macOS`. When you reload a web page, it becomes a new session, and DevTools switches to it on its own. An older page that hasn't closed yet is marked "earlier".

The middle column is the **element tree**: every `View`, `Text`, `Button`, and the rest, with their first classes and text. Collapse and expand branches, or search by kind, class, or text, such as `Button`, `gap-4`, or `Sign in`. Selecting an element highlights it in the app. **Pick** does the reverse: choose an element in the running app, and the tree selects it.

The right column shows one panel at a time:

| Panel     | What you see and do                                                                                                                                                        |
| --------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Node      | The element's kind, its frame, its text, and the file, line, and column that built it, with **Open in editor** (Visual Studio Code). Edit its classes and apply them live. |
| Styles    | Each active class and the CSS it resolved to, such as `p-4` → `padding:1rem;`, and the classes that don't apply at this size or appearance, such as `md:` or `dark:` ones. |
| Route     | The current route, and a field that navigates the app anywhere.                                                                                                            |
| Queries   | The query cache: each key, its status and data. Refetch it, invalidate it, or replace its data to see how the screen handles it.                                           |
| Mutations | Each mutation and its state.                                                                                                                                               |
| Forms     | Each tracked form's values. Read only.                                                                                                                                     |
| State     | The values your app names for DevTools. Edit them as JSON. Values over 64 KiB are shown truncated and can't be edited.                                                     |
| Calls     | Calls to native plugins and to your server, newest first, with their status, duration, and size.                                                                           |
| Commits   | Each render: how many changes it made and, while you profile, how long layout and drawing took.                                                                            |

A panel that a platform can't feed says so. A web page has no native plugin calls, for example, so its Calls panel reads "This session doesn't report plugin or server calls."

When the code of a native app changes, `leptoskit dev` replaces the interface inside the running app. Its state starts over, including values you edited in DevTools. The Calls and Commits timelines mark the moment with "Guest reloaded", and the selected element stays selected when it still exists.

## Showing your app's data

DevTools sees every element on its own, but not your router, your query cache, your forms, or your signals. You hand each one over with a line in `leptoskit::devtools`, behind your app's `devtools` feature like `install`. Outside `leptoskit dev` these calls do nothing, and release builds don't contain them.

This browser app, with [file routes](10-file-routes.md), a [query cache](11-fetching-data.md), and a cart counter, fills the Route, Queries, Mutations, and State panels:

```rust
#[leptos::component]
pub fn App() -> impl leptos::prelude::IntoView {
	use leptos::prelude::*;
	use leptoskit::query::QueryClient;

	let navigation = routes::Navigation::from_browser()
		.unwrap_or_else(|_| routes::Navigation::new(routes::Route::Index));
	let client = QueryClient::new();
	client.provide();
	let cart = RwSignal::new(0_usize);

	#[cfg(feature = "devtools")]
	{
		leptoskit::devtools::track_router(&navigation);
		leptoskit::devtools::track_queries(&client);
		leptoskit::devtools::expose("cart.count", cart);
	}

	move || leptoskit::web::render(navigation.render())
}
```

| Call                           | Panel              | What it shows                                                                                                                                                                                                                                             |
| ------------------------------ | ------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `track_router(&navigation)`    | Route              | The current route, as soon as the app navigates. **Navigate** goes to any route, as a link would.                                                                                                                                                         |
| `track_queries(&client)`       | Queries, Mutations | Each query by its scope and key, with its status, whether it is fetching or stale, how many views use it, when its data arrived, and the data. Each mutation's status. Needs the `query` feature.                                                         |
| `expose("cart.count", signal)` | State              | The signal's value as JSON, with its Rust type. Editing it sets the signal. Only state you name this way appears.                                                                                                                                         |
| `track_form("signup", &state)` | Forms              | A [form](19-forms.md)'s values, its errors, the fields the user has left, and whether a submit was attempted. A `FormState` is a plain value, so call this wherever you have the latest one, such as where you render the form. Needs the `form` feature. |

A query's data is shown and edited through the codec its scope uses for server rendering, so give the scope `.with_ssr()` to see its data. A query without one still shows its status, and its value says why the data is missing. An exposed signal's type needs `Serialize` and `Deserialize`.

DevTools reads these values once your app has settled, not while it renders, and sends only what changed. These calls report from browser apps today. In iOS, Android, and desktop apps they compile and do nothing yet.

## The same element on every platform

When your app runs on several platforms under one `leptoskit dev`, select an element and the Node panel lists its frame on each of them:

```text
Other platforms
web (here)            0, 120 · 390 × 44
iOS · iPhone 17       0, 120 · 390 × 46
Android · Pixel 9     0, 124 · 392 × 44
```

Elements match across platforms by where your code built them, so a button from the same line matches everywhere. **Highlight everywhere** highlights it in every app at once. Browsers and native platforms measure fonts slightly differently, and this is the quickest way to find where that moves your layout.

## Working offline and in CI

The DevTools page is downloaded once per leptoskit version, the first time `leptoskit dev` runs, from the GitHub release of exactly that version. leptoskit checks the download against a fingerprint built into the CLI and refuses anything else, and it never uses another version's page, because the page and the CLI change together.

To download it ahead of time, for example in CI or before going offline:

```bash
leptoskit devtools fetch
```

It is kept in your cache folder: `~/.cache/leptoskit/devtools/<version>/`, or under `$XDG_CACHE_HOME` when that is set. Without it, the hub still runs and shows a plain page with the sessions and their trees, and explains how to download the full one. `leptoskit devtools fetch --force` downloads it again.

## Where it stands

The hub, the page, and `leptoskit devtools fetch` work today. The panel for Chrome, Edge, and Firefox is built, and isn't in their extension stores yet. `leptoskit dev` builds your web page and your macOS, iOS, and Android apps with DevTools turned on, so each one it runs reports itself to the hub. Release builds never include DevTools. Each platform reports:

- **Web:** the element tree, frames, classes, picking, and highlighting, and the route, queries, mutations, forms, and state your app hands over.
- **macOS, iOS, and Android:** the element tree, with each element's kind, classes, text, frame, and the line that built it, and every render in the Commits panel, timed while you profile. **Pick** takes your next tap on a button or another control, and the app never receives that tap; elements that don't respond to taps can't be picked yet. Selecting an element doesn't draw a highlight inside the app yet, and the Styles panel lists a native element's classes without the CSS they resolved to. The route, queries, forms, and state come later. On Android, `leptoskit dev` forwards the hub's port to the emulator or USB device, so it reaches the hub on your machine.
- **Windows, Linux, the terminal, and HarmonyOS (experimental):** not yet.
- **Calls** appear as each part of leptoskit starts reporting them; until then the Calls panel says the app doesn't report them.
