# Examples

Each screen below is one Rust function that returns a tree of leptoskit components, styled with Tailwind classes. The same functions also run in the browser.

The images are real AppKit views, drawn offscreen at 2x by the [`leptoskit_appkit`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_appkit) backend crate, with layout from [`leptoskit_yoga`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_yoga): text is an `NSTextField`, buttons are AppKit controls. The phone screens use a 390×844 viewport. The component studio uses switches, checkboxes, radios, a slider, a password field, and progress bars, which the backend crates draw on every platform but the apps `leptoskit new` generates for macOS, iOS, and Android don't yet; see [platforms](platforms/readme.md#generated-apps-and-backend-crates).

<Gallery>
	<Shot src="https://f002.backblazeb2.com/file/leptoskit/examples/macos/counter.png" alt="A counter screen in the leptoskit palette: a large count of 12, a progress bar at 60%, and a round minus button beside an ember Add one button." title="Counter">A centred card, a progress bar from nested flex boxes, and a pill button that grows to fill the row.</Shot>
	<Shot src="https://f002.backblazeb2.com/file/leptoskit/examples/macos/profile.png" alt="A profile screen with a landscape header, an avatar overlapping it, Message and Follow buttons, a bio, a stats row, and three recent work thumbnails." title="Profile">An avatar positioned over the header, wrapping bio text measured with AppKit's font metrics, and a three-column stats card.</Shot>
	<Shot src="assets/settings.png" alt="A settings screen with a search field, an account card, and grouped rows with coloured icons, values, chevrons, and switches." title="Settings">Grouped rows with inset separators, trailing values and chevrons, and controlled AppKit switches.</Shot>
	<Shot src="https://f002.backblazeb2.com/file/leptoskit/examples/macos/photos.png" alt="A photo library with a segmented control and a grid of landscape thumbnails, one spanning two rows and two columns, with video durations overlaid." title="Photos">CSS grid inside a scroll view, with one item spanning two rows and two columns and duration badges positioned over the tiles.</Shot>
	<Shot src="https://f002.backblazeb2.com/file/leptoskit/examples/macos/chat.png" alt="A chat thread with grey incoming bubbles, indigo outgoing bubbles, an image message, a read receipt, and a message composer." title="Chat">Bubbles capped with `max_width` so text wraps inside them, aligned left or right by `align_self`.</Shot>
	<Shot src="https://f002.backblazeb2.com/file/leptoskit/examples/macos/dashboard.png" alt="A desktop store dashboard with a dark sidebar, four stat cards, a twelve-month revenue bar chart, and a list of top products." title="Dashboard" wide>A 1200×800 desktop layout: a fixed sidebar, a row of stat cards that share the width, a bar chart built from flex items, and a product list.</Shot>
	<Shot src="assets/component-studio.png" alt="The component studio with native input controls, a layered composition, a custom purple button, and a scrolling preview list." title="Component studio" wide>Shared builders compose real native controls, typed layouts, clipping, decorative labels, and an eager scrolling list. The custom pressable supports keyboard and accessibility activation.</Shot>
</Gallery>

## Worked examples

The gallery shows what screens look like. These apps show how the pieces work together, each with tests that drive it the way a person would.

<Gallery>
	<Shot src="assets/observatory.png" alt="The Observatory app: a star catalogue table with sortable headers and a plate pager beside a ribbon of moon-phase night cards above a sky log of observations rated with stars." title="Observatory" wide>A sortable, searchable star catalogue from `leptoskit_table`, beside a year of sky observations and a ribbon of nights, both virtualized lists from `leptoskit_virtual`.</Shot>
	<Shot src="assets/postcard.png" alt="The Postcard app in Arabic, laid out right to left: the card's greeting and lines read from the right, language stamps with ar selected, and a gauge of plural categories per language." title="Postcard" wide>A card home in five languages from ARB files and `leptoskit l10n`, mirrored right to left in Arabic, with a gauge of the plural form each language picks for the same number.</Shot>
</Gallery>

## A screen in a few lines

Each of these screens is built the same way as this card, which is a column on phones and a row from the `md` breakpoint up:

```rust
use leptoskit::prelude::*;

pub fn card() -> Element {
	View()
		.class("gap-3 rounded-xl bg-slate-100 p-4 md:flex-row")
		.children([
			Text("Built once").class("text-xl font-semibold"),
			Button("Continue").class("rounded-lg bg-indigo-500 px-4 py-2 text-white"),
		])
}
```

The [shared component guide](guide/07-shared-components.md) shows how the same function renders natively and on the web. The [CSS coverage reference](reference/css-coverage.md) lists the Tailwind utilities you can use.

## The examples

Every example lives in the leptoskit repository, most under `examples/`:

| Example                                                                                                                       | What it shows                                                                                                                                                               | Where it runs                                                                               |
| ----------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------- |
| [`file_routes`](https://github.com/ifiokjr/leptoskit/tree/main/examples/file_routes)                                          | Field, a notes app whose screens are files: the file router, layouts, typed links, and the whole build pipeline                                                             | Web and every native destination, through the same commands as an app you scaffold yourself |
| [`gallery`](https://github.com/ifiokjr/leptoskit/tree/main/examples/gallery)                                                  | Seven screens on every platform: flexbox, grid, forms, charts, chat bubbles                                                                                                 | Every native backend and the browser; `window` and `snapshot` need macOS                    |
| [`islands`](https://github.com/ifiokjr/leptoskit/tree/main/examples/islands)                                                  | Server rendering with leptos islands, one lazy chunk, and an 80 KiB initial-load budget                                                                                     | Web                                                                                         |
| [`counter`](https://github.com/ifiokjr/leptoskit/tree/main/examples/counter)                                                  | The element tree, layout, mutation diffing, and publishing a code push release                                                                                              | The host, headlessly                                                                        |
| [`observatory`](https://github.com/ifiokjr/leptoskit/tree/main/examples/observatory)                                          | A star catalogue and a year of sky observations: [tables](guide/18-tables.md) and [virtualized lists](guide/17-virtualized-lists.md)                                        | A macOS window, or headlessly on any platform                                               |
| [`postcard`](https://github.com/ifiokjr/leptoskit/tree/main/examples/postcard)                                                | A postcard in five languages: [typed translations](guide/19-localization.md) with plurals, selects, fallback, and right-to-left layout                                      | A macOS window, or headlessly on any platform                                               |
| [`harbor`](https://github.com/ifiokjr/leptoskit/tree/main/examples/harbor)                                                    | A departure board: [queries](guide/11-fetching-data.md), hooks, and [server functions](guide/06-server-functions.md) with a [generated client](guide/13-sharing-the-rpc.md) | The host, headlessly                                                                        |
| [`release_lifecycle`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_codepush/examples/release_lifecycle.rs) | A [code push](guide/05-code-push.md) release from signing to a device that confirms it, a delta, a rollback, and a mirrored bucket                                          | The host, headlessly                                                                        |

## Run them yourself

### file_routes

Field is a complete app in the shape `leptoskit new` produces: one shared application, and runner projects for each platform. From the repository root:

```bash
cd examples/file_routes
leptoskit dev                        # the web app, rebuilding on change
leptoskit build --platform ios       # the six destinations all work
leptoskit plugin:router routes      # regenerate src/routes.rs after adding a screen
```

Its screens are files under `src/routes/`; the [file routes guide](guide/10-file-routes.md) walks through every convention.

### gallery

```bash
cargo run -p gallery -- list                  # the screen names
cargo run -p gallery -- window dashboard      # open one in a real macOS window
cargo run -p gallery -- snapshot screenshots  # render every screen to PNG at 2x
```

`window` and `snapshot` need macOS. On other platforms they say so and exit with an error.

The browser version renders the same seven screens, with working controls. With [Trunk](https://trunkrs.dev/) installed:

```bash
cd examples/gallery
trunk serve
```

### observatory and postcard

```bash
cargo run -p observatory_example                       # open it in a macOS window
cargo run -p observatory_example -- snapshot sky.png   # render it to PNG at 2x on macOS
cargo run -p observatory_example -- summary            # print what a screen reader would read, anywhere
cargo run -p postcard_example -- summary ar            # the postcard, in Arabic
```

The postcard opens in the language your `LANG` names when it has a translation, and in English otherwise. `summary` and `snapshot` take a tag, such as `pl` or `ja`, to choose one.

### harbor

```bash
cargo run -p harbor_example   # a short morning at the harbour, printed as a screen reader reads it
```

Its TypeScript client in `examples/harbor/clients/typescript` is generated. After changing a server function, run `leptoskit rpc generate` in `examples/harbor` to write it again.

### release_lifecycle

```bash
cargo run -p leptoskit_codepush --example release_lifecycle
```

It prints each step of a release as it happens: the store build, a release staged and confirmed, a delta, a rejected release and its fallback, and a mirror to a bucket.
