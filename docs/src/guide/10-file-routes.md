# File routes

Your routes are files. Put one file per screen in `src/routes/`, and leptoskit writes `src/routes.rs`: a Rust module for every file and one `Route` enum with a variant per route. Links are values of that enum, so a link to a route that doesn't exist, or one without its parameters, doesn't compile.

```text
src/
├── lib.rs                  pub mod routes;
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

Then declare the generated module in `lib.rs`. Make it `pub` for now: the router doesn't render pages yet, so `pub` keeps every page reachable instead of dead code.

```rust
pub mod routes;
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

A route file exports its page as a lowercase function, or as a capitalised function annotated with `#[component]`:

```rust
pub fn page() -> Element {
	Text("Notes")
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

A capitalised name without `#[component]`, or both spellings in one file, is an error. Exports other than `page` are recognised and checked now; the router uses them as it gains rendering, data loading and navigation.

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
