# Shared component expansion

- [x] Read the Poteto principles and applicable Rust, devenv, and Git instructions.
- [x] Compare the official React Native core component list and Flutter widget families with the existing renderers.
- [x] Trace native, web, WASM, and testing boundaries with read-only explorers.
- [x] Run the unchanged workspace baseline and capture native gallery output.
- [x] Add shared layout, sizing, clipping, typography, collection, and presentation compositions.
- [x] Add actual native and semantic web controls with typed controlled values and events.
- [x] Extend WASM and widget-test support without changing existing numeric ABI codes.
- [x] Add a working component gallery and a source-linked catalog/support matrix.
- [x] Independently review the implementation, resolve findings, and verify actual native/web artifacts.
- [x] Run formatting, strict lint, workspace tests, browser suites, and native artifact checks.

Delivery uses a two-layer PR stack. Merge each layer only after its platform CI passes, then clean up owned worktrees. GitHub PR state records delivery completion.

Done means the new public builders are exported through `leptoskit::components` and its prelude, accept Tailwind through the existing Element API, and have observable layout/rendering or interaction tests. New interactive primitives must render real AppKit and semantic DOM controls, expose accessible state, preserve controlled ownership and node identity, and round-trip through the WASM host. The shared gallery must exercise the additions. Every advertised component has an explicit support contract and no placeholder implementation. Catalog entries requiring unavailable services are marked deferred.

The estimate is 30–40 additions across two independently verifiable implementation seams: compositions over the existing tree, and genuine controls that extend its backend vocabulary. Existing Element ownership is settled; a second component model or another design arena would add migration work without resolving a new ownership question. Each implementation worker uses its own worktree, with independent review before integration. The available models belong to one family, so the review does not claim model-family diversity.

This is a high-rigor public API and native lifecycle change: exercise state proposals and rejection, disabled/hidden ancestry, keyboard and accessibility semantics, serialization failures, backend identity, and actual visible output. Headless tests do not prove OS input routing or platform support beyond the implemented backends.
