# App-owned motion

`brand.riv` is the original LeptosKit mark, authored in `../../design/brand/scene.rml` and exported with Rive CLI 1.8.0. Rebuild with `rive crates/leptoskit_app/design/brand --once`, verify with `--verify`, then copy the exported file here. The app explicitly selects the `Settle` timeline. It does not depend on a default state machine.

The first and final poses keep all three pieces visible and the mint piece at −12°. The timeline plays once. The shared Rive integration pauses under reduced motion, invisibility, and app inactivity; the public page also supplies a static SVG before its interactive example loads.

`rive.js.gz` and `rive.wasm` are the unmodified, MIT-licensed `@rive-app/canvas@2.45.0` runtime. They match the toolkit's verified browser fixture. The site packager expands the JavaScript into `rive.js`. They are served from the app's own origin and are loaded with the console or when the public example approaches the viewport. The runtime license is in `LICENSE.rive`.
