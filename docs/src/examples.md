# Examples

The gallery composes native primitives with Tailwind and typed styles. Screens return `Element` trees that render through a native surface or the optional `web::render` Leptos adapter. The component studio uses `View` for rows, grids, and layered content, alongside controlled switches, checkboxes, radios, a slider, password input, progress, a custom pressable, and eager scrolling content. It does not require the optional layout helpers.

The images below show AppKit's offscreen render at 2x. [`leptoskit_yoga`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_yoga) computes the native layout, and [`leptoskit_appkit`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_appkit) creates native views. Text uses `NSTextField`, and buttons use AppKit controls.

The phone screens are rendered on macOS at a 390×844 viewport. They show what the layout engine and the renderer can do on AppKit. The UIKit and Android backends render shared element trees on a simulator and an emulator in CI.

<Gallery>
	<Shot src="https://f002.backblazeb2.com/file/leptoskit/examples/macos/counter.png" alt="A counter screen in the leptoskit palette: a large count of 12, a progress bar at 60%, and a round minus button beside an ember Add one button." title="Counter">A centred card, a progress bar from nested flex boxes, and a pill button that grows to fill the row.</Shot>
	<Shot src="https://f002.backblazeb2.com/file/leptoskit/examples/macos/profile.png" alt="A profile screen with a landscape header, an avatar overlapping it, Message and Follow buttons, a bio, a stats row, and three recent work thumbnails." title="Profile">An avatar positioned over the header, wrapping bio text measured with AppKit's font metrics, and a three-column stats card.</Shot>
	<Shot src="assets/settings.png" alt="A settings screen with a search field, an account card, and grouped rows with coloured icons, values, chevrons, and switches." title="Settings">Grouped rows with inset separators, trailing values and chevrons, and controlled AppKit switches.</Shot>
	<Shot src="https://f002.backblazeb2.com/file/leptoskit/examples/macos/photos.png" alt="A photo library with a segmented control and a grid of landscape thumbnails, one spanning two rows and two columns, with video durations overlaid." title="Photos">CSS grid inside a scroll view, with one item spanning two rows and two columns and duration badges positioned over the tiles.</Shot>
	<Shot src="https://f002.backblazeb2.com/file/leptoskit/examples/macos/chat.png" alt="A chat thread with grey incoming bubbles, indigo outgoing bubbles, an image message, a read receipt, and a message composer." title="Chat">Bubbles capped with `max_width` so text wraps inside them, aligned left or right by `align_self`.</Shot>
	<Shot src="https://f002.backblazeb2.com/file/leptoskit/examples/macos/dashboard.png" alt="A desktop store dashboard with a dark sidebar, four stat cards, a twelve-month revenue bar chart, and a list of top products." title="Dashboard" wide>A 1200×800 desktop layout: a fixed sidebar, a row of stat cards that share the width, a bar chart built from flex items, and a product list.</Shot>
	<Shot src="assets/component-studio.png" alt="The component studio with native input controls, a layered composition, a custom purple button, and a scrolling preview list." title="Component studio" wide>Shared builders compose real native controls, typed layouts, clipping, decorative labels, and an eager scrolling list. The custom pressable supports keyboard and accessibility activation.</Shot>
</Gallery>

## Run them yourself

The screens live in [`examples/gallery`](https://github.com/ifiokjr/leptoskit/tree/main/examples/gallery). Each screen is a Rust function returning an `Element`. The gallery's tests also lay out those functions with the headless backend.

For example, a shared screen can express a responsive card without HTML:

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

The [shared component guide](guide/07-shared-components.md) shows native and web entrypoints for the same screen. The [CSS coverage reference](reference/css-coverage.md) records the supported utility families and limits.

```bash
cargo run -p gallery -- list                  # the screen names
cargo run -p gallery -- window dashboard      # open one in a real macOS window
cargo run -p gallery -- snapshot screenshots  # render every screen to PNG at 2x
```

`window` and `snapshot` need macOS. On other platforms they say so and exit with an error.

The browser gallery renders the same seven screen functions, including their control handlers. With [Trunk](https://trunkrs.dev/) installed, start it from its package directory:

```bash
cd examples/gallery
trunk serve
```
