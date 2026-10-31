<Hero start="guide/01-installation.md">

# One leptos tree. Real native widgets. <em>Updated over the air.</em>

leptoskit is a Rust toolkit for building an app's interface once, with leptos components and built-in Tailwind styles, and shipping it to the browser and as real native widgets on the desktop and on phones. When the interface changes, it reaches installed apps through code push, without another trip through the app store.

</Hero>

## What leptoskit is for

Most cross-platform toolkits make you pick: render to a web view everywhere, or write each platform twice. leptoskit takes a third path.

You describe a screen once, in Rust, as a tree of components: `View`, `Text`, `Button`, `TextInput`, `ScrollView`, and the rest. On the web, the same tree renders through leptos to the DOM, with server rendering and hydration. On native platforms, leptoskit's own layout engine lays the tree out with flexbox and CSS grid, and a platform backend commits it as the platform's real controls: AppKit views on macOS, UIKit on iOS, Android views, GTK 4 on Linux, Win32 controls on Windows, and, experimentally, ArkUI on HarmonyOS. In a terminal, the same tree draws as characters in the cell grid. Nothing runs in a web view.

The native interface runs as a small WebAssembly bundle inside an app shell you install once. Publish a new bundle and every installed app can download it, verify every byte against a signed manifest, and try it out. The new version only replaces the working one once it has rendered, so a release that fails to start is rejected and the app keeps running.

<Cards>
	<Card title="Get started" href="guide/01-installation.md">
		Install the CLI, scaffold a project, and see it running in a browser in a few minutes.
	</Card>
	<Card title="Build a shared screen" href="guide/07-shared-components.md">
		Write one screen with the built-in components and Tailwind, and render it on the web and natively.
	</Card>
	<Card title="Ship updates over the air" href="guide/05-code-push.md">
		Pack, sign, publish, and pull new interface versions without an app store release.
	</Card>
	<Card title="Pick your platforms" href="platforms/readme.md">
		What each target runs, what works today, and how the host apps are put together.
	</Card>
</Cards>

## A screen in leptoskit

A screen is a function returning an `Element`. Tailwind classes style it on every platform:

```rust
use leptoskit::prelude::*;

pub fn welcome(name: &str, on_start: impl Fn() + Send + Sync + 'static) -> Element {
	View()
		.class("gap-4 rounded-2xl bg-white p-6 dark:bg-slate-900")
		.children([
			Text(format!("Hello, {name}")).class("text-2xl font-bold"),
			Text("Your interface, everywhere.").class("text-slate-500"),
			Button("Start")
				.class("rounded-lg bg-rose-500 px-4 py-2 text-white")
				.on_press(on_start),
		])
}
```

The same function renders as DOM elements in a browser, and through a platform backend as `NSView`s, `UIView`s, Android views, GTK widgets, Win32 controls, or ArkUI components. [The shared components guide](guide/07-shared-components.md) walks through it, and the [component catalog](reference/components.md) lists every component.

## See it render

These screens are leptoskit element trees committed as real AppKit views and rendered offscreen. They are not mockups. [See all of them](examples.md).

<Gallery>
	<Shot src="https://f002.backblazeb2.com/file/leptoskit/examples/macos/profile.png" alt="A profile screen rendered with native AppKit views" title="Profile">An avatar over the header and wrapping text measured with AppKit's font metrics.</Shot>
	<Shot src="https://f002.backblazeb2.com/file/leptoskit/examples/macos/counter.png" alt="A counter screen rendered with native AppKit views" title="Counter">A centred card, a progress bar of nested flex boxes, and a pill button.</Shot>
	<Shot src="https://f002.backblazeb2.com/file/leptoskit/examples/macos/chat.png" alt="A chat screen rendered with native AppKit views" title="Chat">Bubbles capped with `max_width`, aligned by `align_self`.</Shot>
</Gallery>

## How the pieces fit

The logo is the architecture. The ember boxes are layout: elements placed by the layout engine and committed as native widgets. The mint piece is the next version of your interface arriving over the air and dropping into place. Hover over it at the top of this page.

| Crate                                                                                            | What it does                                                                                                                                                                                                                                                                                                                                                                                                                               |
| ------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| [`leptoskit`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit)                   | The crate your app depends on. It re-exports the components, styles, and runtime, with a feature per optional part.                                                                                                                                                                                                                                                                                                                        |
| [`leptoskit_cli`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_cli)           | The `leptoskit` command, also installed as `lk`, which scaffolds, builds, serves, tests, formats, and ships apps.                                                                                                                                                                                                                                                                                                                          |
| [`leptoskit_native`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_native)     | The components, Tailwind support, and the renderer that diffs element trees into mutation batches for a platform backend.                                                                                                                                                                                                                                                                                                                  |
| [`leptoskit_yoga`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_yoga)         | The layout engine behind native rendering: flexbox and CSS grid, gaps, aspect ratio, and right-to-left layout.                                                                                                                                                                                                                                                                                                                             |
| [`leptoskit_appkit`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_appkit)     | The macOS backend. [`leptoskit_uikit`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_uikit), [`leptoskit_android`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_android), [`leptoskit_gtk`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_gtk), and [`leptoskit_windows`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_windows) do the same for their platforms. |
| [`leptoskit_wasm`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_wasm)         | The on-device runtime. It runs the interface bundle in `wasmi`, an interpreter, so it is allowed on iOS.                                                                                                                                                                                                                                                                                                                                   |
| [`leptoskit_codepush`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_codepush) | Over-the-air updates: signed, content-hashed releases, binary deltas, and a client that verifies everything before it stages.                                                                                                                                                                                                                                                                                                              |
| [`leptoskit_axum`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_axum)         | The server toolkit: static files with precompressed copies, SPA fallback, health checks, and the code push registry, on Axum.                                                                                                                                                                                                                                                                                                              |
| [`leptoskit_router`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_router)     | File-based routing. `src/routes/` becomes one typed `Route` enum.                                                                                                                                                                                                                                                                                                                                                                          |
| [`leptoskit_query`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_query)       | Cached, deduplicated, retrying async data in the style of TanStack Query.                                                                                                                                                                                                                                                                                                                                                                  |
| [`leptoskit_l10n`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_l10n)         | Typed translations in the style of Flutter's `gen_l10n`: ARB files become a `Messages` trait, with plurals for every language.                                                                                                                                                                                                                                                                                                             |
| [`leptoskit_testing`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_testing)   | Widget tests with semantic finders, plus a driver that runs the same journeys against real native apps.                                                                                                                                                                                                                                                                                                                                    |
| [`leptoskit_docs`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_docs)         | Markdown with leptos components, and the generator that builds this site. See [writing docs](guide/14-writing-docs.md).                                                                                                                                                                                                                                                                                                                    |

## Where things stand

leptoskit is young, and these docs say plainly what works today.

Native filesystem, SQLite, and secure-store plugins enforce aggregate storage budgets in addition to operation limits. See [storage and networking](platforms/plugin-inventory.md#storage-and-networking) for defaults and upgrade behavior.

<Callout kind="note" title="Works now">

The web pipeline: cached builds, `wasm-opt`, server rendering, islands, and Axum serving. The layout engine and the renderer. Native backends for macOS, iOS, Android, Linux, Windows, and the terminal that draw every built-in component, checked in CI on a real Mac, an iOS simulator, an Android emulator, a Linux desktop, a Windows desktop, and headless terminal sessions. Generated apps for all of those platforms. The Wasm runtime and code push, from a signed release to a verified, staged update on iOS and Android. File routes, server functions, queries, translations, and the testing tools.

</Callout>

<Callout kind="important" title="Known gaps">

- **Generated macOS, iOS, and Android apps** draw nine components: `View`, `Text`, `Button`, `Pressable`, `TextInput`, `ScrollView`, `Image`, `Vector`, and `Rive`. Any other component stops them with an "unsupported widget" error. They pass on each element's accessible name; `Rive` also supports decorative hiding. A labelled `Vector` is announced as an image. Other roles and states are not passed on. The backend crates for those platforms draw everything; the generated apps don't use them yet. The generated Windows and Linux apps draw every component.
- **iOS backend:** iOS has no native checkbox or radio button, so `Checkbox` and `Radio` are drawn with `UIButton`. VoiceOver gets fewer roles than macOS, and no expanded or collapsed state.
- **Android backend:** a `Custom` widget kind with no registered native implementation stops the app instead of rendering an empty container. TalkBack gets a smaller set of roles than macOS.
- **Linux:** live regions update their text but are not announced, because announcements need GTK 4.14 and leptoskit supports GTK 4.6.
- **Windows:** `Switch` is drawn as a checkbox. Rounded corners are cut with a window region, so their edges are not smoothed, and borders stay rectangular inside them. Accessibility has no expanded or collapsed state. A `Vector` with no background takes clicks only where it has ink.
- **Desktop shell plugin:** invocation limits and cancellation clean up Unix process groups and Windows Job Objects. Unix commands that deliberately leave their group need additional host containment. Windows retains capacity when a member's termination cannot be confirmed; see the [plugin inventory](platforms/plugin-inventory.md).
- **Rive:** [shared Rive elements](guide/20-rive.md) render on the web, macOS, iOS, Android, Linux, Windows, and experimental HarmonyOS. Native playback supports vector scenes and rejects images, fonts, text, layout, scripts, audio, view models, and unsupported draw features. View-model data binding is not exposed. HarmonyOS playback still needs validation on the target device.
- **Runtime plugins on Windows, Linux, and the terminal:** the generated apps don't register runtime plugins yet. Plugins listed in `native.plugins` are left out with a warning, and calls to them fail with `Unavailable`. See [plugins a platform can't run](platforms/host-projects.md#plugins-a-platform-cant-run).
- **HarmonyOS (experimental):** eight components (not including `Vector`), synchronous Rust plugins and ArkTS plugins (fold posture is the only ArkTS one so far) but not worker or async plugins, and no code push, dark mode, right-to-left layout, `active:` pressed styles, or CI run on a device. See [HarmonyOS](platforms/harmonyos.md).
- **Terminal:** the terminal's font is the user's, so font sizes and weights draw as attributes and colors rather than different metrics, `Image` and `Vector` draw a placeholder texture, `Rive` playback is unsupported, and right-to-left text is not reordered. See [Terminal](platforms/tui.md).
- **Styled text, italic, per-edge borders, key handlers, and controlled focus** work on the web and in the terminal. The native widget backends draw a `Text`'s styled runs in its own single style, draw italic text upright, leave a per-edge border such as `border-l-2` undrawn, deliver no `on_key` presses, and do not move focus for `focused(true)`.

On every native platform, `Image` loads local files only, not URLs. Code push updates reach the iOS and Android apps; the generated desktop apps don't check for them yet. CSS shadows, transforms, gradients, and custom fonts aren't supported anywhere yet: the [CSS coverage reference](reference/css-coverage.md) lists what is. Rive's internal vector paints and transforms are separate from those CSS properties. [Platforms](platforms/readme.md) has the full picture.

</Callout>

Axum is the one supported server.

To work on leptoskit itself, start with [CONTRIBUTING.md](https://github.com/ifiokjr/leptoskit/blob/main/CONTRIBUTING.md).
