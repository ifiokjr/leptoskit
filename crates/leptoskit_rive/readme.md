# leptoskit_rive

Play an app-owned `.riv` file on a browser canvas, with typed state-machine inputs and an owned player that releases Rive when dropped. The optional `leptoskit` features `rive` and `web` expose it as `leptoskit::rive`.

The asset API works on native, browser Wasm, and WASI. **Rendering currently targets browsers only.** AppKit, UIKit, Android, GTK, Windows, and HarmonyOS widgets are not implemented. A native app must not substitute a `Custom` element and expect it to render Rive.

## Browser setup

Pin an official runtime such as `@rive-app/canvas@2.45.0`, bundle its JavaScript with your app, and serve its matching Wasm file from your origin. The crate neither injects a script nor fetches a runtime from a CDN. Pass the imported namespace to `Runtime::new` once, before another part of the app initializes that namespace. Reuse it across players. WebGL2 runtime namespaces use the same high-level API, but the integration tests exercise Canvas2D.

```rust,ignore
use leptoskit::rive::{Asset, Input, Name, Scene};
use leptoskit::rive::web::{Options, Player, Runtime};

// The page has already loaded the matching self-hosted /assets/rive.js script.
let runtime = Runtime::from_global("/assets/rive.wasm")?;
let asset = Asset::new(include_bytes!("../public/status.riv"))?;
let options = Options {
    scene: Scene::StateMachine(Name::new("Status")?),
    autoplay: true,
    ..Options::default()
};
// Attach and give the canvas a CSS width and height before mounting.
canvas.set_attribute("aria-label", "Upload status")?;
let player = Player::new(&runtime, &canvas, &asset, &options)?;
player.ready().await?;
player.set_input(&Name::new("progress")?, Input::Number(50.0))?;
```

Hold the player for as long as the canvas is mounted. With Leptos, store it in owner-local state and drop that state in the owner's cleanup; the player is a browser object and must stay on its creating thread. `ready()` reports parser failures, missing scenes, rejected external assets, and a 15-second load timeout. Drop during a pending import also disposes its resources. `close()` additionally returns upstream cleanup errors; Drop reports cleanup errors to the browser console.

The canvas must have an `aria-label`, or `aria-hidden="true"` when decorative. It needs a definite CSS size so its intrinsic pixel size does not drive a resize loop. Keep meaningful status and interactive controls in accessible HTML beside it. File-authored pointer/keyboard listeners, automatic URL events, and audio are disabled. Use `set_input` to drive named number, boolean, and trigger inputs from app-owned controls; missing names and type mismatches return errors.

## Performance and trust

- An `Asset` owns shared immutable bytes. Cloning it does not copy the file. A player copies bytes across the JS boundary once at mount; frames stay inside the upstream runtime.
- The runtime schedules frames. Pause, page visibility, intersection, and reduced motion stop rendering. Reduced motion preserves the initial pose. Changing the preference or becoming visible resumes only if playback was requested.
- The backing canvas uses at most 4,194,304 pixels, 4,096 pixels on either axis, and a device scale of 2. The player restores the previous backing dimensions on disposal. This bounds its canvas allocation, not all runtime memory or GPU memory.
- Files must embed their fonts, images, and audio. The Rive asset CDN and public Wasm fallback are disabled, and the asset loader rejects missing embedded data. There is no asset URL loader or unbounded global file cache.
- Encoded files are limited to 8 MiB before copying. Header checks reject truncated, overflowing, and unsupported version fields. **These checks do not validate the object graph, bound decompressed resources, authenticate content, or sandbox scripts.** Use only files you ship or authenticate yourself, and keep the upstream runtime patched. Do not feed arbitrary uploads into a page's main-thread runtime. A load timeout cannot interrupt synchronous parser or script execution.

## Verification

`cargo test -p leptoskit_rive` covers the portable boundaries. `devenv shell test:rive` also runs real offline Rive playback in Chrome and checks lifecycle and policy behavior. Test fixtures pin the official Canvas runtime at 2.45.0 under its MIT license; they are excluded from the published package. The tiny animation is authored in `tests/fixtures/scene/scene.rml`; rebuild with `rive tests/fixtures/scene --once` from this crate, then verify, inspect, and render before committing the new `.riv`.
