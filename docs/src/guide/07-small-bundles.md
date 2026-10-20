# Small bundles

Every byte of wasm is a byte the browser downloads, compiles, and runs before your page responds, and in a native app it is a byte that travels over the air with each update. leptos gives you two big levers for shipping less: render on the server so the client only hydrates, and hydrate only the islands that are interactive. leptoskit makes each one a line in `leptoskit.toml`, compiles the result as small as it goes, and tells you what you shipped.

## Pick a render mode

```toml
[app]
render = "islands"
```

| `render`    | What renders where                                              | What the wasm carries           |
| ----------- | --------------------------------------------------------------- | ------------------------------- |
| `"csr"`     | the browser renders everything                                  | the whole app                   |
| `"ssr"`     | the server renders every page, the browser hydrates all of it   | every component                 |
| `"islands"` | the server renders every page, the browser hydrates the islands | only the `#[island]` components |

`csr` is the default, so nothing changes until you opt in. The two server-rendering modes follow the convention every leptos app already uses: leptoskit builds the server package with its `ssr` feature and the wasm client with its `hydrate` feature, and in islands mode turns on `leptos/islands` for both. Since the mode is set in one place, the server and the client can't disagree about it.

## An islands app

The [`islands` example](https://github.com/ifiokjr/leptoskit/tree/main/examples/islands) is a complete app in one crate: the library is the wasm client and the binary is the server.

```toml
[lib]
crate-type = ["cdylib", "rlib"]

[[bin]]
name = "my_app"
path = "src/main.rs"
required-features = ["ssr"]

[dependencies]
leptos = { version = "0.8", features = ["islands"] }
leptos_router = "0.8"
serde = { version = "1", features = ["derive"] }
wasm-bindgen = "=0.2.129"
axum = { version = "0.8", optional = true }
leptos_axum = { version = "0.8", optional = true }
leptoskit_axum = { version = "0.1", optional = true }
tokio = { version = "1", features = ["rt-multi-thread", "macros"], optional = true }

[features]
ssr = [
	"dep:axum",
	"dep:leptos_axum",
	"dep:leptoskit_axum",
	"dep:tokio",
	"leptos/ssr",
	"leptos_router/ssr",
]
hydrate = ["leptos/hydrate"]
```

`render = "islands"` turns `leptos/islands` on for leptoskit's builds whatever the manifest says. Keep it in `Cargo.toml` too, because `cargo check` and rust-analyzer don't read `leptoskit.toml`, and `#[island]` doesn't compile without it. Island props cross from the server to the client as JSON, so the crate depends on `serde` directly.

The client entry point hydrates the islands, and nothing else:

```rust,ignore
#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
	console_error_panic_hook::set_once();
	leptos::mount::hydrate_islands();
}
```

The shell asks `HydrationScripts` for the islands loader:

```rust,ignore
<HydrationScripts options islands=true />
```

The server renders the routes and serves the built site behind them with [`leptoskit_axum::static_files`](https://docs.rs/leptoskit_axum/latest/leptoskit_axum/fn.static_files.html):

```rust,ignore
let options = get_configuration(None)?.leptos_options;
let site_root = std::path::PathBuf::from(options.site_root.as_ref());
let app = Router::new()
	.leptos_routes(&options, generate_route_list(App), {
		let options = options.clone();
		move || shell(options.clone())
	})
	.fallback_service(leptoskit_axum::static_files(&site_root))
	.with_state(options);
```

`leptoskit dev` and `leptoskit serve` start that binary with `LEPTOS_OUTPUT_NAME`, `LEPTOS_SITE_ROOT`, `LEPTOS_SITE_PKG_DIR`, `LEPTOS_SITE_ADDR`, and `LEPTOS_ENV` set, so `get_configuration(None)` finds the built site with no `[package.metadata.leptos]` table. To run it yourself in production, set the same variables: `LEPTOS_SITE_ROOT` is the `--out` directory and `LEPTOS_OUTPUT_NAME` is the client's library name.

Mark a component `#[island]` only when it needs to run in the browser. Everything else, including the components that wrap islands, renders on the server and never reaches the wasm.

## Release builds

`leptoskit build --release` makes the client as small as it can without any setup:

- **A size-tuned cargo profile.** The wasm client compiles with `leptoskit-wasm`, a profile leptoskit defines on the command line: your `release` profile with `opt-level = "z"`, fat LTO, one codegen unit, and `panic = "abort"`. Your `Cargo.toml` doesn't change, and the server keeps your own `release` profile. To use a profile of your own instead, name it: `build.wasm_profile = "wasm-release"`.
- **`wasm-opt -Oz`**, binaryen's most aggressive size optimization, when `build.wasm_opt` is on (the default).
- **No debugging sections.** The name and producers sections are dropped from release wasm.
- **Precompressed output.** Every compressible file (wasm, JavaScript, CSS, HTML, SVG, JSON) gets a brotli `.br` and a gzip `.gz` copy, compressed once at the highest settings instead of on every request. `leptoskit serve`, `leptoskit_axum::static_files`, and most CDNs send them to browsers that accept them. Turn it off with `build.precompress = false`. Dev builds remove the copies, so a server never sends a stale one.

## See what you ship

Every release build ends with the client bundle report. Here's the `islands` example:

```text
client bundle:
                                      raw        gzip      brotli
  pkg/islands_example.js         25.8 KiB     5.5 KiB     4.8 KiB
  pkg/islands_example_bg.wasm   163.5 KiB    71.8 KiB    61.1 KiB
  initial load                  189.3 KiB    77.3 KiB    65.9 KiB
```

The **initial load** is everything the page downloads before it can respond. Set a budget for it and the build holds the line:

```toml
[build]
budget_kib = 80
```

A release build whose initial load is heavier than 80 KiB brotli-compressed fails, after writing its output, with the size it reached. Run `leptoskit build --release` in CI and a pull request that bloats the bundle can't merge unnoticed.
