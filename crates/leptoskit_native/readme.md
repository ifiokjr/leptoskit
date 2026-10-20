<p align="center">
  <a href="https://ifiokjr.github.io/leptoskit/"><img src="https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

# leptoskit_native

> The leptoskit native renderer: React Native's architecture, rewritten in Rust.

Apps describe their interface as a tree of `Element`s. Each commit, the renderer diffs the new description against the committed shadow tree and emits a minimal batch of mutations — create, update props, insert, remove, delete — which a platform `NativeBackend` applies to real native widgets. Layout runs through [`leptoskit_yoga`], and only changed frames are pushed back. Events bubble from widgets through handlers like the DOM.

There is no web view anywhere in the pipeline. Every element maps one-to-one onto a native widget (`UIView` on iOS, an Android view, an `NSView` on macOS), which is what makes the renderer suitable for code-pushed wasm bundles driving platform UI.

## The pipeline

1. `NativeSurface::set_root(&element_tree)` — the app commits a description.
2. Diff — the surface walks old and new trees, producing a `MutationList`.
3. Apply — the backend turns mutations into widget calls, batched.
4. Layout — yoga computes frames; only changed frames become `UpdateLayout` mutations.
5. Events — `dispatch_event` bubbles from the target through ancestors.

## Backends

- `HeadlessBackend` records every mutation; it powers tests, snapshot suites, and the wasm host's recording mode.
- Platform backends implement `NativeBackend` and map mutations onto the platform widget toolkit. Text measurement plugs in through `TextMeasurer` so layout matches the platform's font metrics.

## Relationship to leptos

A leptos component's `view!` is a description of an interface; this renderer is the reconciliation half. The `leptoskit` meta crate re-exports everything here, and the wasm host (`leptoskit_wasm`) drives a `NativeSurface` from a code-pushed bundle so the same component tree runs on web and native.
