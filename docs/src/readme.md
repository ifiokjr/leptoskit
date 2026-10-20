<div class="lk-hero">
	<div class="lk-mark" aria-hidden="true">
		<span class="lk-slot"></span>
		<span class="lk-tall"></span>
		<span class="lk-square"></span>
		<span class="lk-ota"></span>
	</div>
	<div>
		<p class="lk-hero-title">One leptos tree.<br>Real native widgets.<br><em>Updated over the air.</em></p>
		<p class="lk-hero-lede">leptoskit is the toolkit for building, deploying, and shipping leptos apps: to the browser with SSR and hydration, and to iOS, Android, and the desktop as real platform widgets, no web view.</p>
		<div class="lk-hero-actions">
			<a class="lk-button lk-button-primary" href="guide/01-installation.html">Get started</a>
			<a class="lk-button lk-button-secondary" href="https://github.com/ifiokjr/leptoskit">GitHub</a>
		</div>
	</div>
</div>

# Introduction

leptoskit takes the architecture React Native proved, with one component tree and a native host per platform, and rebuilds it in Rust from the diff down to the layout engine. The interface you write is ordinary leptos. On the web it renders to the DOM. Everywhere else it runs as a wasm module that drives native widgets, and that wasm module is the same artifact you push to devices over the air.

<div class="lk-cards">
	<div class="lk-card">
		<span class="lk-card-glyph"></span>
		<h3>Native, not a web view</h3>
		<p>Every element maps to a platform widget through a small backend contract. Apps get platform accessibility, input methods, and behaviour for free.</p>
	</div>
	<div class="lk-card">
		<span class="lk-card-glyph"></span>
		<h3>Layout from scratch</h3>
		<p>A Rust layout engine with flexbox and CSS grid, so layout numbers agree on every platform because it is the same code everywhere.</p>
	</div>
	<div class="lk-card">
		<span class="lk-card-glyph lk-mint"></span>
		<h3>Code push built in</h3>
		<p>Content-hashed releases, binary deltas, and a client that verifies every byte before staging the next interface version.</p>
	</div>
	<div class="lk-card">
		<span class="lk-card-glyph"></span>
		<h3>A fast CLI</h3>
		<p>Every build phase fingerprints its inputs, so a rebuild after a one-line change runs only the phases whose inputs moved.</p>
	</div>
</div>

## See it render

These are leptoskit element trees committed as real AppKit views, not mockups. [See all six examples](examples.md).

<div class="lk-showcase">
	<a href="examples.html"><img src="https://f002.backblazeb2.com/file/leptoskit/examples/macos/profile.png" alt="A profile screen rendered with native AppKit views" loading="lazy" width="390" height="844"></a>
	<a href="examples.html"><img src="https://f002.backblazeb2.com/file/leptoskit/examples/macos/counter.png" alt="A counter screen rendered with native AppKit views" loading="lazy" width="390" height="844"></a>
	<a href="examples.html"><img src="https://f002.backblazeb2.com/file/leptoskit/examples/macos/chat.png" alt="A chat screen rendered with native AppKit views" loading="lazy" width="390" height="844"></a>
</div>

## How the pieces fit

The logo is the architecture. The ember boxes are layout: elements arranged by the layout engine and committed as native widgets. The mint piece is the next version of your interface arriving over the air and dropping into place.

| Crate                                                                                            | What it is                                                                                                      |
| ------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------- |
| [`leptoskit_yoga`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_yoga)         | The layout engine: flexbox and CSS grid (`fr`, `minmax`, spans, dense auto-placement), gaps, RTL, aspect ratio. |
| [`leptoskit_native`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_native)     | The renderer: element trees diff into mutation batches that a platform backend applies to real widgets.         |
| [`leptoskit_appkit`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_appkit)     | The macOS backend: mutation batches committed as AppKit views, text measured with AppKit's font metrics.        |
| [`leptoskit_wasm`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_wasm)         | The on-device runtime: runs the app's wasm in `wasmi`, an interpreter, so it is allowed on iOS.                 |
| [`leptoskit_codepush`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_codepush) | Over-the-air updates: hashed manifests, a single-file bundle container, binary deltas, routes, and a client.    |
| [`leptoskit_axum`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_axum)         | The server toolkit: static files with precompressed gzip, SPA fallback, health checks, the release registry.    |
| [`leptoskit_fns`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_fns)           | Retrying, error-classified queries and mutations over leptos server functions.                                  |
| [`leptoskit_config`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_config)     | `leptoskit.toml`: app identity, platforms, servers, native targets, code push, and plugins.                     |
| [`leptoskit_plugin`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_plugin)     | Build hooks for leptoskit plugins, and leptos plugins forwarded as app cargo features.                          |
| [`leptoskit_cli`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_cli)           | The `leptoskit` binary: `new`, `dev`, `build`, `serve`, `codepush`, `doctor`, `completions`.                    |
| [`leptoskit`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit)                   | The meta crate that re-exports everything above.                                                                |

## Where things stand

leptoskit is early, and these docs say plainly what works today:

- **Works now:** the web pipeline (cached builds, wasm-bindgen, `wasm-opt`, Axum serving), the layout engine, the renderer with its headless and AppKit backends, the wasm runtime, and code push from release to staged update. The [`gallery`](https://github.com/ifiokjr/leptoskit/tree/main/examples/gallery) example renders six screens as real macOS views. The [`platform_probe`](https://github.com/ifiokjr/leptoskit/tree/main/examples/platform_probe) suite runs that whole stack in CI on macOS, Linux, Windows, and the iOS simulator, and cross-compiles it for Android.
- **In progress:** the platform backends that turn mutation batches into UIKit, Android, WinUI, and GTK widgets, plus the C ABI that a host shell links. `leptoskit new --platform native` already scaffolds the Swift Package host that the bridge will attach to.

Only Axum is supported as the server.

## Next steps

- [Install the CLI](guide/01-installation.md) and [create a project](guide/02-creating.md).
- Read [the platform guides](platforms/readme.md) for how each target runs your interface.
- See [the brand page](brand.md) for logo files and colours.
