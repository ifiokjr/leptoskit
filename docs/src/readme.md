<div class="lk-hero">
	<div class="lk-mark" aria-hidden="true">
		<span class="lk-slot"></span>
		<span class="lk-tall"></span>
		<span class="lk-square"></span>
		<span class="lk-ota"></span>
	</div>
	<div>
		<p class="lk-hero-title">Shared Rust screens.<br>Real native widgets.<br><em>Updated over the air.</em></p>
		<p class="lk-hero-lede">Build screens with leptoskit components and built-in Tailwind styles. Render the same tree with Leptos on the web or native AppKit views on macOS. Additional native backends use the shared integration contract.</p>
		<div class="lk-hero-actions">
			<a class="lk-button lk-button-primary" href="guide/01-installation.html">Get started</a>
			<a class="lk-button lk-button-secondary" href="https://github.com/ifiokjr/leptoskit">GitHub</a>
		</div>
	</div>
</div>

# Introduction

Build your interface with `View()`, `Text()`, `Button()`, and the other shared Rust builders. Each screen returns an `Element` tree with built-in Tailwind classes. A native surface resolves that tree to layout and widget mutations. The optional `web::render` adapter creates Leptos DOM views from the same tree.

The [shared component guide](guide/07-shared-components.md) shows both entrypoints. The [CSS coverage reference](reference/css-coverage.md) records the supported utility subset and its limits. Native hosts can run their interface bundle through the wasm runtime and receive code push updates.

<div class="lk-cards">
	<div class="lk-card">
		<span class="lk-card-glyph"></span>
		<h3>Native, not a web view</h3>
		<p>The AppKit backend maps shared controls to macOS widgets. The NativeBackend contract defines widget operations for additional platforms.</p>
	</div>
	<div class="lk-card">
		<span class="lk-card-glyph"></span>
		<h3>Layout from scratch</h3>
		<p>A Rust layout engine handles native flexbox, CSS grid, and box sizing. Web rendering emits CSS and uses the browser's layout engine.</p>
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

These are leptoskit element trees committed as real AppKit views, not mockups. [See all seven examples](examples.md).

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
| [`leptoskit_rpc`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_rpc)           | Retrying, error-classified queries and mutations over leptos server functions.                                  |
| [`leptoskit_config`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_config)     | `leptoskit.toml`: app identity, platforms, servers, native targets, code push, and plugins.                     |
| [`leptoskit_plugin`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_plugin)     | Build hooks for leptoskit plugins, and leptos plugins forwarded as app cargo features.                          |
| [`leptoskit_cli`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_cli)           | The `leptoskit` binary: `new`, `dev`, `build`, `serve`, `codepush`, `doctor`, `completions`.                    |
| [`leptoskit`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit)                   | The meta crate that re-exports everything above.                                                                |
| [`leptoskit_testing`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_testing)   | Native widget tests with semantic finders, interactions, bounded pumping, and diagnostic trees.                 |

## Where things stand

leptoskit is early, and these docs say plainly what works today:

- **Works now:** the web pipeline (cached builds, wasm-bindgen, `wasm-opt`, Axum serving), the layout engine, the renderer with its headless and AppKit backends, the wasm runtime, and code push from release to staged update. The [`gallery`](https://github.com/ifiokjr/leptoskit/tree/main/examples/gallery) example renders seven screens as real macOS views. The [`platform_probe`](https://github.com/ifiokjr/leptoskit/tree/main/examples/platform_probe) suite runs that whole stack in CI on macOS, Linux, Windows, and the iOS simulator, and cross-compiles it for Android.
- **In progress:** the platform backends that turn mutation batches into UIKit, Android, WinUI, and GTK widgets, plus the C ABI that a host shell links. `leptoskit new --platform native` already scaffolds the Swift Package host that the bridge will attach to.

Only Axum is supported as the server.

## Next steps

- [Install the CLI](guide/01-installation.md) and [create a project](guide/02-creating.md).
- [Build a shared screen](guide/07-shared-components.md) with the built-in components and Tailwind.
- Read [the platform guides](platforms/readme.md) for how each target runs your interface.
- See [the brand page](brand.md) for logo files and colours.
