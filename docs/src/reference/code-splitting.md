# Code splitting across web and native

Web code splitting is built into leptoskit. A common Rust API for web and native CodePush is feasible, but native chunk loading is not implemented in the production host. This reference separates what works today from the proposed API and the work needed to support it.

The source review uses the workspace's locked Leptos 0.8.14, Leptos macro 0.8.19, Leptos Router 0.8.10, wasm-split CLI support 0.2.4, wasm-split helpers 0.2.3, and wasmi 2.0.0. See [the lockfile](../../../Cargo.lock).

## What works today

| Capability                                | Web                                                  | Native CodePush                                         |
| ----------------------------------------- | ---------------------------------------------------- | ------------------------------------------------------- |
| Build splitting enabled by default        | Yes; clients without lazy imports skip the transform | No; the native guest has a separate build pipeline      |
| Function split points                     | Leptos `#[lazy]`                                     | Not integrated into `WasmHost`                          |
| Lazy Leptos routes                        | `#[lazy_route]` with `LazyRoute`                     | Leptos views are not the native guest UI ABI            |
| Lazy islands                              | `#[island(lazy)]`                                    | Not applicable to native rendering                      |
| Generated file routes split automatically | No                                                   | No                                                      |
| Signed release containing multiple files  | Not required by the web loader                       | Yes, but launch hands the runtime only the entry module |

These are separate systems: the [web build](../../../crates/leptoskit_cli/src/build.rs) runs wasm-split before wasm-bindgen, while the [native build](../../../crates/leptoskit_cli/src/native/mod.rs) copies a raw ABI guest to `app.wasm`. The [file-route generator](../../../crates/leptoskit_router_codegen/src/emit.rs) currently emits direct calls to every page through a synchronous [`Render::render`](../../../crates/leptoskit_router/src/router.rs) returning `Element`.

### Web defaults

There is no configuration needed to use supported Leptos split points. `bundle.split` defaults to `true`; the build detects `__wasm_split_placeholder__` imports and transforms only clients that actually contain lazy code. Setting `split = false` is an explicit opt-out, and a build with lazy imports then fails with a diagnostic. See [`BundleConfig`](../../../crates/leptoskit_config/src/lib.rs), [the build pipeline](../../../crates/leptoskit_cli/src/build.rs), and [split detection](../../../crates/leptoskit_cli/src/split.rs).

Standalone clients discard the linker's `linking` and `reloc.*` custom sections before optimization. Keeping those build-time sections prevents Binaryen from optimizing the module normally; unrelated application custom sections remain intact. See [the binding phase](../../../crates/leptoskit_cli/src/wasm.rs) and [its regression tests](../../../crates/leptoskit_cli/tests/split.rs).

For example, this is the existing Leptos API:

```rust,ignore
#[leptos::lazy]
fn decode_report(input: String) -> Report {
	Report::decode(input)
}

let report = decode_report(input).await;
```

Both synchronous and asynchronous annotated functions produce asynchronous calls. Parameters and results must use concrete types; generics and `impl Trait` are not supported at this boundary. Subsequent calls reuse the loaded code, but still return a future. See [Leptos `lazy`](https://docs.rs/leptos_macro/0.8.19/leptos_macro/attr.lazy.html).

The build keeps relocation information, splits reachable code into modules, writes the JavaScript loader and `__wasm_split_manifest.json`, then binds the main module. Release builds optimize the main module and chunks. A function also reachable from eager code cannot be assumed to leave the main module just because a lazy function uses it. See [leptoskit's splitter](../../../crates/leptoskit_cli/src/split.rs) and [wasm-split's dependency analysis](https://docs.rs/wasm_split_cli_support/0.2.4/src/wasm_split_cli_support/split_point.rs.html).

For SSR, keep `<MetaTags/>` and `<HydrationScripts/>` in the shell so Leptos can preload split points used during the current page's server render. This helps avoid discovering its code only after hydration begins. A lazy island loads as it hydrates; it is not automatically deferred until scrolling into view. The [small-bundles guide](../guide/09-small-bundles.md) describes the working setup and its limits.

Enabled splitting does not choose useful split points for an application. Keep startup, navigation, and the visible shell eager; defer optional screens and heavy tools. For content-heavy web pages, islands can remove static rendering code from the client altogether before splitting the remaining interactive code. Islands require a server and a different hydration model, so they should remain an explicit rendering choice. See [Leptos's islands guide](https://book.leptos.dev/islands.html).

### Current failure semantics

Leptos `#[lazy]` does not enable wasm-split's `fallible` option. A chunk loading failure therefore panics, even when the annotated function happens to return an application `Result`. The return type alone does not make loading recoverable. The macro's [implementation](https://docs.rs/leptos_macro/0.8.19/src/leptos_macro/lazy.rs.html) delegates to the lower-level helper without that option.

The underlying [wasm-split helper](https://docs.rs/wasm_split_helpers/0.2.3/wasm_split_helpers/attr.wasm_split.html) supports `fallible`, an explicit split group, and an optional named preload function. Fallible functions preserve `Result<_, E>` when `E` converts from `SplitLoaderError`; preload returns a loading result. A failed load can be attempted again. This is useful machinery for a framework API, but is not a new API exported by leptoskit.

## Can the same API work natively?

Yes at the source level. The split helper asks an imported function to load a group, passing a callback and opaque guest data. A loader reports success through that callback. The browser implements this with JavaScript; a native host can implement the import directly. Chunks import the main module's shared memory, indirect function table, and other required exports. This makes a function boundary with ordinary Rust values possible without inventing a JSON RPC boundary. See the [helper runtime](https://docs.rs/wasm_split_helpers/0.2.3/src/wasm_split_helpers/rt.rs.html) and [splitter exports](https://docs.rs/wasm_split_cli_support/0.2.4/src/wasm_split_cli_support/emit.rs.html).

wasmi already exposes instance exports, memory, and tables, and resolves imports between modules through a linker. Those exports must belong to the same store. This provides the required building blocks; it does not supply leptoskit's chunk lifecycle or CodePush integration. See [wasmi `Instance`](https://docs.rs/wasmi/2.0.0/wasmi/struct.Instance.html) and [`Linker`](https://docs.rs/wasmi/2.0.0/wasmi/struct.Linker.html).

The repository includes an explicitly invoked [native feasibility probe](../../../crates/leptoskit_cli/tests/native_split.rs) and a [real Rust guest](../../../crates/leptoskit_cli/tests/fixtures/split_guest/src/lib.rs). It compiles the guest, runs the real splitter, and exercises the callback protocol in wasmi without JavaScript. Its assertions cover deferred loading, a failed load followed by retry, successful load reuse, and a `String` allocated through shared guest memory. The probe passed on the development host on 22 October 2026. It runs in the wasm end-to-end CI step and is ignored in ordinary test runs.

```sh
cargo test --locked -p leptoskit_cli --test native_split -- --ignored
```

On macOS versions affected by [Rust's stripped proc-macro loading issue](https://github.com/rust-lang/rust/issues/157750), run this with `CARGO_PROFILE_RELEASE_BUILD_OVERRIDE_STRIP=none`. That environment setting was needed for the recorded run; it changes host build dependencies, not the guest's splitting behavior.

This does not promise identical web and native binaries. Browser bundles use wasm-bindgen and may depend on DOM imports; the native guest uses the `leptoskit` host ABI. Plain Leptos lazy routes return `AnyView`, whereas leptoskit's cross-platform file routes return a shared `Element` tree. A common source-level loading contract can select different target bindings and build artifacts. See [the web binding phase](../../../crates/leptoskit_cli/src/wasm.rs), [the native host ABI](../../../crates/leptoskit_wasm/src/lib.rs), and [the shared component facade](../../../crates/leptoskit/src/lib.rs).

## Recommended public API

This section is a design recommendation, not shipped API. Exact macro and helper syntax should be settled with compiling web and native examples before release.

Prefer a small framework wrapper over a second independent splitter:

| Approach                                                            | Benefit                                               | Limit                                                                               |
| ------------------------------------------------------------------- | ----------------------------------------------------- | ----------------------------------------------------------------------------------- |
| Reuse Leptos `#[lazy]`, `#[lazy_route]`, and lazy islands directly  | Existing web examples and SSR preloading keep working | Browser-oriented macro behavior; loading failures panic; no native host integration |
| Add a thin leptoskit function annotation over the same split helper | One typed, fallible call contract across targets      | Requires target-aware expansion, SSR integration, and native runtime work           |
| Introduce arbitrary dynamically linked feature plugins              | Independent deployment boundaries                     | A new ABI and compatibility model far beyond reducing initial load                  |

Retain the existing Leptos APIs for compatibility. Add a wrapper only once both target loaders and their failure paths are implemented. Start with one annotation, one loading error, and one typed preload operation; keep URLs, release manifests, and native host handles out of application call sites.

For example, a future API could express this intent:

```rust,ignore
#[leptoskit::lazy]
async fn decode_report(input: String) -> Result<Report, ReportError> {
	Report::decode(input).await
}

let report = decode_report(input).await?;
```

`ReportError` would explicitly convert from the framework's loading error, keeping both the feature's errors and code loading failures in one declared `Result`. Require concrete boundary types and report unsupported signatures at the annotation. Preserve `async` in the source so readers can see that a first call may wait for code. On an ordinary non-Wasm build, run the function directly with the same call and result shape. The annotation should support explicit grouping only when several entry points need to share a loading boundary; omit manual chunk names from the common case.

A typed preload operation should take a function path or generated route value, load its code without executing the feature, and return `Result<(), LoadError>`. It should not require a function's business arguments. Keep loading policy separate from the loading mechanism: callers can preload on navigation intent or during idle time without implementing another cache.

### Loading, errors, retry, and cancellation

The proposed contract should make these behaviors explicit:

- Concurrent callers share one in-flight load per group and release. Once loaded, calls execute their own function bodies; only code loading is deduplicated.
- Successful code stays available for the life of the guest. A transient failed attempt does not permanently poison the group. Retrying must be explicit or bounded by the caller's policy.
- Loading failure returns an error before the feature body runs. The error identifies the group and supports useful diagnostics without exposing a platform-specific exception type. The current callback carries only success/failure, so richer transport or compatibility detail requires additional loader diagnostics.
- Dropping a caller's future stops waiting for that caller. It does not unload code, roll back another caller, or guarantee aborting a shared fetch. The host must still complete every issued callback exactly once, even after navigation changes.
- A completed stale navigation must not replace the current screen. Route state needs a generation or equivalent ownership rule, and disposal must release reactive state associated with the abandoned view.
- Loading UI remains usable and accessible. Keep the navigation shell interactive, announce a pending destination, and offer retry for recoverable failures. Avoid blanking the entire application while a leaf screen loads.

The existing lower-level runtime and JavaScript loader provide a starting point for success caching and retry; cancellation, native callback scheduling, error presentation, and route ownership need framework-level verification. See [the helper runtime](https://docs.rs/wasm_split_helpers/0.2.3/src/wasm_split_helpers/rt.rs.html) and [the loader implementation](https://docs.rs/wasm_split_cli_support/0.2.4/src/wasm_split_cli_support/snippets/split_wasm.js.html).

## File routes as the default boundary

File routes are a sensible future automatic boundary: developers already describe one screen per file. Keep route matching, typed parameters, shared layouts, and loading/error presentation in the initial module; split each leaf page's rendering entry point. Private helper modules follow reachability. An explicit eager override handles tiny screens or startup-critical flows. This is a proposed generator/runtime change; the [current generator](../../../crates/leptoskit_router_codegen/src/emit.rs) does not do it.

Start route data work as soon as the destination is known, concurrently with loading its view code. If a fetch starts only inside the lazy view body, navigation becomes a code-then-data waterfall. Leptos already models this through [`LazyRoute::data`, `view`, and `preload`](https://docs.rs/leptos_router/0.8.10/leptos_router/trait.LazyRoute.html); [`#[lazy_route]`](https://docs.rs/leptos_router/0.8.10/leptos_router/attr.lazy_route.html) explicitly loads the view concurrently with its data. Reuse that separation where compatible rather than introducing a competing web resource model.

The shared `Element` file router needs its own asynchronous route state before adopting this default. Preserve its current synchronous rendering API until that state has a clear loading/error contract and a migration path. A blanket annotation on generated page calls would hide loading state and break its current return type.

For the initial URL, discover the selected route's chunks early: SSR can emit preload links; CSR needs route matching in the entry module. Never preload every route by default, because that gives back the download saving. User intent, such as focusing or hovering a link, is a reasonable later prefetch policy, with network and platform policy kept outside the basic function API.

## CodePush rollout

Ship local deferred instantiation first: download and verify all chunks as one release, then instantiate a group only when called. This preserves offline feature availability and the existing update selection/rollback model. It does not reduce the full release's download size. The current installer expands the complete container before returning the entry module, so packaged chunks do not defer that expansion either. See [container parsing and launch](../../../crates/leptoskit_codepush/src/installed.rs).

Native startup has a different baseline from the browser: wasmi defaults to lazy function translation, and `WasmHost` uses that default configuration. A single module already defers translation of cold functions. Splitting may defer some module validation, metadata, or instantiation work, but it must be measured against that baseline; the probe establishes no native speedup. See [wasmi's compilation mode](https://docs.rs/wasmi/2.0.0/wasmi/struct.Config.html#method.compilation_mode) and [host configuration](../../../crates/leptoskit_wasm/src/lib.rs).

The [container format](../../../crates/leptoskit_codepush/src/manifest.rs) already stores multiple files with hashes. The [installed release manager](../../../crates/leptoskit_codepush/src/installed.rs) verifies compatibility and authenticates downloaded releases, but `LaunchBundle` exposes only the entry Wasm bytes. The [production host](../../../crates/leptoskit_wasm/src/lib.rs) instantiates one module and does not link split-loader imports. The following work remains:

1. Split native guests before packaging, preserving the required relocation and shared exports without running browser wasm-bindgen glue.
2. Include a versioned split descriptor inside the authenticated release. Record groups, ordered dependencies, chunk files, and the loader compatibility requirement; use the existing per-file hashes rather than a second release identity.
3. Hand the host a release-bound, verified source for both entry and chunk bytes. Resolve every group against the running release, never a mutable “latest” directory or another release's chunks.
4. Keep the main and chunk instances in one wasmi store, share their required exports, and queue loader completion through the serialized guest runtime. Extend fuel and resource accounting to chunk instantiation and callbacks.
5. Preserve activation receipts and rollback. Old asynchronous completions must not run against a replacement guest, and chunk failure must produce a defined recoverable state or reject an incompatible release.
6. Exercise a signed update, offline relaunch, navigation, first use, failed load/retry, cancellation, guest replacement, and rollback on real supported hosts before enabling native splitting by default.

The current [`RuntimeContract`](../../../crates/leptoskit_codepush/src/installed.rs) rejects unknown fields. Adding a split-loader capability there requires a deliberate host compatibility change; it cannot be silently added to releases intended for old shells.

Remote download on first use is a later protocol. It needs a signed complete dependency graph, release-pinned chunk hashes, bounded verified downloads, cache retention, offline behavior, and a rule for a release whose unused chunks were never downloaded. Keep that delivery policy behind the same function/preload API. Do not make optional feature availability depend on a network connection as an accidental consequence of enabling code splitting.

## Evidence required before calling it faster

The current bundle report distinguishes main assets from lazy files, but excluded lazy bytes are not proof of a faster first screen: the selected route or island may need them immediately. See [bundle reporting](../../../crates/leptoskit_cli/src/build.rs) and [the small-bundles guide](../guide/09-small-bundles.md).

Compare the split build with an equivalent eager baseline: remove or gate the lazy annotations while preserving the same functionality and build profile. Setting `split = false` alone cannot supply that baseline, because the build correctly refuses unresolved lazy imports. Record the first route's actual requested bytes, first meaningful frame, time to interaction, cold navigation, and repeat navigation. On native, separately measure installed offline launch, CodePush download, entry instantiation, and first lazy feature use. Include loading UI and failure behavior in that comparison. This investigation establishes an implementation direction; it does not establish a measured native startup improvement.
