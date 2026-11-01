# File routes

Your routes are files. Put one file per screen in `src/routes/`, and leptoskit writes `src/routes.rs`: a Rust module for every file and one `Route` enum with a variant per route. Links are values of that enum, so a link to a route that doesn't exist, or one without its parameters, doesn't compile.

```text
src/
├── lib.rs                    mod routes;
├── routes.rs                 generated, committed
└── routes/
    ├── _layout.rs            chrome around every screen
    ├── +not-found.rs         anything unmatched
    ├── (tabs)/
    │   ├── index.rs          /
    │   ├── map.rs            /map?near=
    │   └── notes/
    │       ├── index.rs      /notes
    │       ├── _note_card.rs a private module, never a route
    │       └── [note].rs     /notes/:note
    ├── help/[...topic].rs    /help/*topic
    └── (auth)/sign-in.rs     /sign-in
```

Turn the router on with a `[router]` table in `leptoskit.toml`:

```toml
[router]
dir = "src/routes" # the default
```

The generated module calls `leptoskit_router` by name, so the app depends on it directly. Turn on its `web` feature in web builds, so the router follows the address bar:

```toml
[dependencies]
leptoskit_router = "0.1"

[features]
web = ["leptoskit/web", "leptoskit_router/web"]
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
| `_note_card.rs`        | a private module beside the routes that use it                        |                       |
| `+not-found.rs`        | URLs nothing else matches, at or below its directory                  |                       |
| `+html.rs`             | the server-rendered document shell; it lives at the top of the routes |                       |

The generator picks every Rust module name, so file names don't have to be Rust identifiers: `[note].rs` becomes `pub mod note` through `#[path]`, and `sign-in.rs` becomes `pub mod sign_in`. A name that still isn't one is escaped the same way for its module and its `Route` variant: a leading digit gets a leading `_`, and a Rust keyword gets a trailing `_`. So `404.rs` is `pub mod _404` and `Route::_404`, and `self.rs` is `pub mod self_` and `Route::Self_`; declare a `#[derive(Route)]` struct to give such a route a better name. Parameter names become struct fields, so they are identifiers to begin with: `[note_id]`, not `[note-id]`.

When two routes could match a URL, the more specific one wins, segment by segment: a static segment beats a parameter, which beats a catch-all. An optional segment counts as present or absent, whichever the URL has. `/about` matches `[[lang]]/about.rs` with no language, as a static segment, before `[section].rs` can read it as a section; `/fr/about` starts with a parameter, so a `fr/[page].rs` would win that one. Two files that match exactly the same URLs, such as `(shop)/cart.rs` and `(store)/cart.rs`, or `[[a]]/[b].rs` and `[c]/[[d]].rs`, are an error.

## Items are behaviour

Prefer a PascalCase `#[component]` function for pages and layouts. The router also accepts lowercase plain functions for compatibility. It returns anything that implements `IntoView`:

```rust
#[component]
pub fn Page() -> Element {
	view! { <Text>"Notes"</Text> }
}
```

A layout wraps every route in its directory and below. It takes `children` and places them; the router passes the page, or the layout nested inside it:

```rust
#[component]
pub fn Layout(children: Children) -> Element {
	view! { <View class="flex-1 bg-white dark:bg-slate-950">{children}</View> }
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

Only `pub` items are exports: a private function is the file's own helper, whatever its name. A capitalised export without `#[component]`, both spellings in one file, or a `_layout.rs` without a `layout`, is an error. The router renders `page` and `layout` today; the other exports are recognised and checked now, and the router uses them as it gains data loading and navigators.

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

- `Option<T>` may be absent and is left out of the URL when it is `None`; an optional segment like `[[lang]]` needs one. A field named after a segment only ever reads from the path, so `/about?lang=fr` has no language.
- `Vec<T>` takes every value: the segments of a catch-all like `[...topic]`, or each repeat of a query key.
- Any other `T` is required, unless the field is `#[route(default)]`, which reads `T::default()` when the value is absent.

Every `T` implements `FromStr` and `Display`. The path decides whether a route matches: a path value that doesn't parse means the URL doesn't match that route, so `/notes/draft` falls through to `+not-found.rs` when `note` is a number. A query value that doesn't parse is ignored, as if it were absent, so `/notes/42?highlight=x` is note 42 with no highlight. A required query field, one that isn't an `Option`, a `Vec` or `#[route(default)]`, still needs a value that parses, or the route doesn't match. Route structs derive `Clone`, `Debug` and `PartialEq`, because the `Route` enum does.

A static route doesn't need a struct; its name comes from its path, so `/sign-in` is `Route::SignIn` and `/` is `Route::Index`. Declare a unit struct to choose another name: `pub struct Today;` in `(tabs)/index.rs` makes `/` `Route::Today`.

## Using the routes

```rust
use leptoskit_router::Routable;

use crate::routes::Compose;
use crate::routes::Note;
use crate::routes::Route;

let route = Route::parse("/notes/42?highlight=3")?;
assert_eq!(
	route,
	Route::Note(Note {
		note: 42,
		highlight: Some(3)
	})
);

let reply = Route::from(Compose { reply_to: Some(42) });
assert_eq!(reply.href().as_str(), "/compose?reply_to=42");
```

`Route::parse` reads any URL the way a browser does, so a link someone typed or shared always reaches a route: a stray `%`, as in `/map?near=100%`, stays as it is, escapes that aren't UTF-8 become `�`, and `.` and `..` segments are resolved.

A URL path can't hold an empty segment, `.` or `..`: browsers drop or resolve them. So `href` never writes one, and never starts with `//`, which would name another site. An optional segment holding such a value is left out, so `About { lang: Some(String::new()) }` links to `/about`; a required segment or catch-all value holding one ends the path there, so `Section { section: String::new() }` links to `/settings`.

`routes.rs` also checks that each route struct still has a field for every segment of its file name. Rename `[note].rs` without renaming the field, and the build fails with "`Note` needs a field named `note`" before anything runs.

## Rendering and navigating

`routes.rs` defines `Navigation`, the app's router. It holds the current route, and `render` returns that route's page inside its layouts:

```rust
use crate::routes::Navigation;
use crate::routes::Note;
use crate::routes::Route;

let navigation = Navigation::new(Route::Notes);
let screen = navigation.render();

navigation.navigate(Note {
	note: 42,
	highlight: None,
}); // remembers /notes
navigation.replace(Route::You); // in place
navigation.back(); // returns to /notes
```

A link is a pressable that navigates. On the web it renders as an `<a href>`, so it can open in a new tab and crawlers can follow it, while a plain click stays in the app:

```rust
#[component]
pub fn Page(navigation: &Navigation) -> Element {
	navigation
		.link(
			Note {
				note: 42,
				highlight: None,
			},
			"Open note 42",
		)
		.child(view! { <Text>"Note 42"</Text> })
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

`from_browser` fails only when the page's URL matches no route, which can't happen with a root `+not-found.rs`. The router assumes the app is served from the root of its site: hrefs start with `/`, and the whole path is matched against the routes, so serving the app under a sub-path such as `/app/` isn't supported yet.

## Generating `routes.rs`

`leptoskit dev`, `leptoskit build` and `leptoskit native` regenerate `routes.rs` before they compile, and write it only when the routes changed. After adding a route outside those commands, run the router plugin's command:

```bash
leptoskit plugin:router routes
```

It writes the module and lists every route, most specific first. The file is committed, so a fresh clone compiles with plain `cargo build`, and rust-analyzer sees ordinary Rust. In CI, fail when someone forgot to regenerate it:

```bash
leptoskit plugin:router routes --check
```

Formatting doesn't count as a change, so rustfmt can format `routes.rs` like any other file. leptoskit never overwrites a `routes.rs` it didn't write.

Per-platform route files and folders, such as `index.web.rs` or `home.ios.rs`, are planned as an opt-in extension and are rejected for now.
