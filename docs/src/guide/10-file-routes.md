# File routes

Your routes are files. Put one file per screen in `src/routes/`, and leptoskit writes `src/routes.rs`: a Rust module for every file and one `Route` enum with a variant per route. Links are values of that enum, so a link to a route that doesn't exist, or one without its parameters, doesn't compile.

```text
src/
├── lib.rs                  mod routes;
├── routes.rs               generated, committed
└── routes/
    ├── _layout.rs          chrome around every screen
    ├── +not-found.rs       anything unmatched
    ├── (tabs)/
    │   ├── index.rs        /
    │   ├── map.rs          /map?near=
    │   └── notes/
    │       ├── index.rs    /notes
    │       ├── _card.rs    a private module, never a route
    │       └── [note].rs   /notes/:note
    ├── help/[...topic].rs  /help/*topic
    └── (auth)/sign-in.rs   /sign-in
```

Turn the router on with a `[router]` table in `leptoskit.toml`:

```toml
[router]
dir = "src/routes" # the default
```

Then declare the generated module in `lib.rs`:

```rust
mod routes;
```

[`examples/file_routes`](https://github.com/ifiokjr/leptoskit/tree/main/examples/file_routes) is a complete app built this way.

## Names are URLs

| Name                   | Meaning                                                               | URL                   |
| ---------------------- | --------------------------------------------------------------------- | --------------------- |
| `index.rs`             | the directory's own route                                             | `/notes`              |
| `map.rs`, `sign-in.rs` | a static segment; kebab-case is fine                                  | `/sign-in`            |
| `[note].rs`, `[note]/` | one dynamic segment                                                   | `/notes/42`           |
| `[[lang]]/`            | an optional segment                                                   | `/about`, `/fr/about` |
| `[...topic].rs`        | one or more segments                                                  | `/help/sync/ios`      |
| `[[...topic]].rs`      | any number of segments                                                | `/help`, `/help/sync` |
| `(tabs)/`              | a group: it shapes the tree and adds nothing to the URL               |                       |
| `_layout.rs`           | the directory's layout                                                |                       |
| `_card.rs`             | a private module beside the routes that use it                        |                       |
| `+not-found.rs`        | URLs nothing else matches, at or below its directory                  |                       |
| `+html.rs`             | the server-rendered document shell; it lives at the top of the routes |                       |

The generator picks every Rust module name, so file names don't have to be Rust identifiers: `[note].rs` becomes `pub mod note` through `#[path]`.

When two routes could match a URL, the more specific one wins, segment by segment: a static segment beats a parameter, which beats an optional parameter, which beats a catch-all. Two files that match exactly the same URLs, such as `(shop)/cart.rs` and `(store)/cart.rs`, are an error.

## Items are behaviour

A route file exports its page as a lowercase function, or as a capitalised function annotated with `#[component]`. It returns anything that implements `IntoView`:

```rust
pub fn page() -> Element {
	Text("Notes")
}
```

A layout wraps every route in its directory and below. It takes `children` and places them; the router passes the page, or the layout nested inside it:

```rust
pub fn layout(children: Children) -> Element {
	View()
		.class("flex-1 bg-white dark:bg-slate-950")
		.children(children)
}
```

The generator reads these names from each file without running it:

| Item                                        | In           | Does                                         |
| ------------------------------------------- | ------------ | -------------------------------------------- |
| `page` or `#[component] Page`               | route files  | the screen; required                         |
| `layout` or `#[component] Layout`           | `_layout.rs` | chrome around the directory's routes         |
| `loading`, `error`, `head` (or capitalised) | any          | pending UI, the error boundary, web metadata |
| `html` or `#[component] Html`               | `+html.rs`   | the document shell                           |
| `#[derive(Route)] pub struct`               | route files  | the route's name and typed parameters        |
| `load`, `prerender`                         | route files  | data and static parameters                   |
| `navigator`, `redirect`                     | `_layout.rs` | the directory's navigator and guard          |

A capitalised name without `#[component]`, or both spellings in one file, is an error. The router renders `page` and `layout` today; the other exports are recognised and checked now, and the router uses them as it gains data loading and navigators.

### Props

Pages and layouts receive props by name. Declare the ones you need, in any order, by reference or by value:

| Prop         | Page                                | Layout               |
| ------------ | ----------------------------------- | -------------------- |
| `route`      | the route's struct, such as `&Note` | the whole `Route`    |
| `navigation` | `&Navigation`, to navigate and link | `&Navigation`        |
| `children`   |                                     | `Children`; required |

A page whose route has no `#[derive(Route)]` struct has no `route` to receive. Any other parameter name is an error that names the file and line.

## Typed parameters

A route with parameters declares one struct with `#[derive(Route)]`. Its name becomes the route's variant. A field named after a `[segment]` comes from the path; every other field comes from the query string:

```rust
use leptoskit_router::Route;

/// `src/routes/(tabs)/notes/[note].rs`, matching `/notes/:note`.
#[derive(Route, Clone, Debug, PartialEq)]
pub struct Note {
	pub note: u64,              // the [note] segment
	pub highlight: Option<u32>, // ?highlight=
}
```

A field's type decides how it reads:

- `Option<T>` may be absent and is left out of the URL when it is `None`; an optional segment like `[[lang]]` needs one.
- `Vec<T>` takes every value: the segments of a catch-all like `[...topic]`, or each repeat of a query key.
- Any other `T` is required, unless the field is `#[route(default)]`, which reads `T::default()` when the value is absent.

Every `T` implements `FromStr` and `Display`. A value that doesn't parse means the URL doesn't match that route: `/notes/draft` falls through to `+not-found.rs` when `note` is a number. Route structs derive `Clone`, `Debug` and `PartialEq`, because the `Route` enum does.

A static route doesn't need a struct; its name comes from its path, so `/sign-in` is `Route::SignIn` and `/` is `Route::Index`. Declare a unit struct to choose another name: `pub struct Today;` in `(tabs)/index.rs` makes `/` `Route::Today`.

## Using the routes

```rust
use leptoskit_router::Routable;

use crate::routes::{Compose, Note, Route};

let route = Route::parse("/notes/42?highlight=3")?;
assert_eq!(route, Route::Note(Note { note: 42, highlight: Some(3) }));

let reply = Route::from(Compose { reply_to: Some(42) });
assert_eq!(reply.href().as_str(), "/compose?reply_to=42");
```

`routes.rs` also checks that each route struct still has a field for every segment of its file name. Rename `[note].rs` without renaming the field, and the build fails with "`Note` needs a field named `note`" before anything runs.

## Rendering and navigating

`routes.rs` defines `Navigation`, the app's router. It holds the current route, and `render` returns that route's page inside its layouts:

```rust
use crate::routes::{Navigation, Note, Route};

let navigation = Navigation::new(Route::Notes);
let screen = navigation.render();

navigation.navigate(Note { note: 42, highlight: None }); // remembers /notes
navigation.replace(Route::You);                         // in place
navigation.back();                                      // returns to /notes
```

A link is a pressable that navigates. On the web it renders as an `<a href>`, so it can open in a new tab and crawlers can follow it, while a plain click stays in the app:

```rust
pub fn page(navigation: &Navigation) -> Element {
	navigation
		.link(
			Note {
				note: 42,
				highlight: None,
			},
			"Open note 42",
		)
		.child(Text("Note 42"))
}
```

The current route is a reactive signal. Natively, the host renders again after each event, so give it the router's render function: `UIKitHost::new(mtm, move || navigation.render())`. Earlier routes stay in memory for `back`.

In the browser, turn on `leptoskit_router`'s `web` feature and start from the page's URL. The router then adds a history entry for each navigation and follows the back and forward buttons:

```rust
let navigation = Navigation::from_browser()?;
leptos::mount::mount_to_body(move || {
	let navigation = navigation.clone();
	move || leptoskit::web::render(navigation.render())
});
```

## Generating `routes.rs`

`leptoskit dev`, `leptoskit build` and `leptoskit native` regenerate `routes.rs` before they compile, and write it only when the routes changed. After adding a route outside those commands, run:

```bash
leptoskit routes
```

It writes the module and lists every route, most specific first. The file is committed, so a fresh clone compiles with plain `cargo build`, and rust-analyzer sees ordinary Rust. In CI, fail when someone forgot to regenerate it:

```bash
leptoskit routes --check
```

Formatting doesn't count as a change, so rustfmt can format `routes.rs` like any other file. leptoskit never overwrites a `routes.rs` it didn't write.

Per-platform route files and folders, such as `index.web.rs`, are planned as an opt-in extension and are rejected for now.
