# leptoskit_rive

Play an app-owned `.riv` file with typed state-machine inputs and an owned player that releases Rive when dropped. The `native` feature provides CPU vector rendering; `web` provides browser canvas playback with an app-supplied official runtime. The optional `leptoskit` feature `rive` exposes it as `leptoskit::rive` and enables the shared `Rive` element.

The asset API works on native, browser Wasm, and WASI. Native widgets use AppKit, UIKit, Android images, GTK textures, Win32 bitmaps, and experimental HarmonyOS PixelMaps. Rendering is not available inside a WASI guest: the native host owns the runtime outside its interpreter.

## Shared elements

```rust,ignore
use leptoskit::components::Rive;
use leptoskit::rive::Asset;
use leptoskit::rive::Playback;
use leptoskit::view;

let playback = Playback::new(Asset::new(include_bytes!("status.riv"))?);
let element = view! {
	<Rive rive_playback=playback class="w-32 h-32" aria_label="Upload status" running=true />
};
```

Scenes start paused. Give the element a definite size and a direct accessible label, or use `aria_hidden(true)` for decoration. Reconciliation retains the scene until the asset, artboard, or selection changes. `Playback::set_input` validates typed updates; a fresh sequence repeats a trigger, while an unchanged sequence prevents unrelated renders from firing it again. Removing a binding preserves its last applied value.

Generated native hosts schedule frames and stop when idle, offscreen, inactive, or under reduced motion. A host installed before this feature needs a native rebuild before a code-pushed guest can use Rive. A custom native embedding must call `NativeSurface::tick_rive` or `tick_rive_with` on its UI thread after a commit or viewport/preference change, and then only while the result requests another frame. Borrowed frames must be copied or uploaded synchronously, without reentering the surface. HarmonyOS's host uses the application-supplied `AppStorage` preference `leptoskit.rive.reduceMotion`; device playback still needs validation on the chosen target.

In a browser root's Leptos owner, call `leptoskit::web::provide_rive_runtime(runtime)` before rendering shared elements. The browser player is released with its canvas state. Errors reach the console and `data-lk-rive-error` on the canvas. The directly owned browser player below provides explicit error handling instead.

## Native feature coverage

The pinned upstream C++ runtime and tiny-skia rasterizer render animated vector paths, fills, centered strokes, linear/radial gradients, and path clips. Native rendering rejects images, fonts, text, Rive layout, scripts, audio, view models, feathering, angular gradients, and unsupported drawing operations. This is a vector renderer; it does not implement all Rive editor features or view-model data binding. Use vector-only files for assets shared with native apps.

Native builds need a C++17 compiler and the target SDK. Sources are pinned and checksummed in `leptoskit_rive_runtime`; builds perform no runtime downloads. `native::Player` is thread-confined and reuses its backing buffer and cached paths. Its initial render draws a pose even while paused; further idle renders return `None`. Each frame is tightly packed premultiplied RGBA8888, bounded to 4,194,304 pixels and 4,096 pixels per axis. Surface-managed players additionally share an 8,388,608-pixel budget, cap scale at 2, and allow at most 16 players. These raster limits do not bound all C++ decoded object memory or authenticate a file. Only import trusted assets.

## Browser setup

Pin an official runtime such as `@rive-app/canvas@2.45.0`, bundle its JavaScript with your app, and serve its matching Wasm file from your origin. The crate neither injects a script nor fetches a runtime from a CDN. Pass the imported namespace to `Runtime::new` once, before another part of the app initializes that namespace. Reuse it across players. WebGL2 runtime namespaces use the same high-level API, but the integration tests exercise Canvas2D.

```rust,ignore
use leptoskit::rive::Asset;
use leptoskit::rive::Input;
use leptoskit::rive::Name;
use leptoskit::rive::Scene;
use leptoskit::rive::web::Options;
use leptoskit::rive::web::Player;
use leptoskit::rive::web::Runtime;

// The page has already loaded the matching self-hosted /assets/rive.js
// script.
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

`cargo test -p leptoskit_rive` covers portable boundaries. `devenv shell test:rive` adds native pixel rendering, retained-surface integration, real offline browser playback, and lifecycle policies. Test fixtures pin the official Canvas runtime at 2.45.0 under its MIT license; fixtures are excluded from the published package. The animation and static paint fixture are authored under `tests/fixtures/scene/` and `tests/fixtures/native/`. Rebuild with the Rive CLI, then verify, inspect, and render before committing exported files. Native pixel output is retained under `target/rive-native/` for comparison with the CLI renderer.
