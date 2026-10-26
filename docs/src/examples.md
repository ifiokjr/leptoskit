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

Every example lives in the leptoskit repository under `examples/`:

| Example                                                                              | What it shows                                                                                                   | Where it runs                                                                               |
| ------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------- |
| [`file_routes`](https://github.com/ifiokjr/leptoskit/tree/main/examples/file_routes) | Field, a notes app whose screens are files: the file router, layouts, typed links, and the whole build pipeline | Web and every native destination, through the same commands as an app you scaffold yourself |
| [`gallery`](https://github.com/ifiokjr/leptoskit/tree/main/examples/gallery)         | Seven screens on every platform: flexbox, grid, forms, charts, chat bubbles                                     | Every native backend and the browser; `window` and `snapshot` need macOS                    |
| [`islands`](https://github.com/ifiokjr/leptoskit/tree/main/examples/islands)         | Server rendering with leptos islands, one lazy chunk, and an 80 KiB initial-load budget                         | Web                                                                                         |
| [`counter`](https://github.com/ifiokjr/leptoskit/tree/main/examples/counter)         | The element tree, layout, mutation diffing, and publishing a code push release                                  | The host, headlessly                                                                        |

## Run them yourself

### file_routes

Field is a complete app in the shape `leptoskit new` produces: one shared application, and runner projects for each platform. From the repository root:

```bash
cd examples/file_routes
leptoskit dev                        # the web app, rebuilding on change
leptoskit build --platform ios       # the six destinations all work
leptoskit routes                     # regenerate src/routes.rs after adding a screen
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
