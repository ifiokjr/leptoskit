# Rive animations

Use Rive when a status illustration needs to move or respond to your application's state. The shared `Rive` element plays an app-owned `.riv` file on the web, macOS, iOS, Android, Windows, Linux, and experimental HarmonyOS. A paused player keeps its current pose visible. Removing the element releases its renderer.

Add `rive` to your app crate's `leptoskit` features. This example assumes a vector illustration with a `Status` state machine and a numeric `progress` input:

```rust,ignore
use leptoskit::components::Rive;
use leptoskit::native::Element;
use leptoskit::rive::{Asset, Error, Input, Name, Playback, Scene};

fn upload_status(asset: &Asset, progress: f64) -> Result<Element, Error> {
    let mut playback = Playback::new(asset.clone())
        .scene(Scene::StateMachine(Name::new("Status")?));
    playback.set_input(Name::new("progress")?, Input::Number(progress), 0)?;
    Ok(Rive(playback)
        .class("w-40 h-40")
        .aria_label("Upload progress")
        .running(true))
}
```

Load the `Asset` once with `Asset::new(include_bytes!("../assets/upload.riv"))?`, and share it across screen updates. Give the element a definite width and height. Native hosts retain the parsed scene when its asset, artboard, and selected scene stay the same, even when labels or input values change. Input-only guest updates reuse the asset already installed on that node. A repeated `Input::Trigger` needs a new sequence in `set_input(name, value, sequence)`; unrelated screen updates do not fire it again. Removing an input update leaves its last value in the retained scene.

On native platforms the host rasterizes vector paths and presents them in native image widgets, without a web view. Frames stay outside the app's Wasm guest. The native renderer supports animated paths, fills, centered strokes, linear and radial gradients, and path clips. It rejects images, fonts, text, Rive layout, scripts, audio, view models, and drawing features it cannot reproduce, including feathering and angular gradients. Selected state machines must use unique input names. Use a vector-only file for an illustration shared with native platforms. View-model data binding is not exposed on any platform.

Newly generated hosts for those native platforms include playback scheduling. Existing app-owned hosts need the updated Rive integration before a code-pushed guest can use it. The terminal backend does not support Rive playback. HarmonyOS remains experimental, with device playback requiring validation on your target. Its host accepts the app's reduced-motion setting through `AppStorage` key `leptoskit.rive.reduceMotion` (default `false`).

## Set up the browser runtime

The shared element renders a browser canvas, including on server-rendered pages. Before rendering it in the browser's root Leptos owner, supply the app-owned runtime once:

```rust,ignore
let runtime = leptoskit::rive::web::Runtime::from_global("/assets/rive.wasm")?;
leptoskit::web::provide_rive_runtime(runtime);
```

Load the matching self-hosted JavaScript before that call. The element mounts after DOM insertion, retains its player through reactive updates, and drops it when removed. Import and input failures are reported to the browser console and the canvas's `data-lk-rive-error` attribute. Use the low-level player below when you need to handle errors directly.

## Own a browser player directly

Enable `rive` and `web` on `leptoskit`, or depend directly on `leptoskit_rive` with its `web` feature. Bundle a pinned official Rive runtime, such as `@rive-app/canvas@2.45.0`, with your application's JavaScript and serve its matching Wasm file from your own origin. Pass that imported namespace into `Runtime::new` once at startup, before using it elsewhere.

For an upload illustration whose state machine is named `Status` and whose number input is named `progress`, the browser code is:

```html
<script src="/assets/rive.js"></script>
```

```rust,ignore
use leptoskit::rive::{Asset, Input, Name, Scene};
use leptoskit::rive::web::{Options, Player, Runtime};

let runtime = Runtime::from_global("/assets/rive.wasm")?;
let asset = Asset::new(include_bytes!("../assets/upload.riv"))?;
let options = Options {
    scene: Scene::StateMachine(Name::new("Status")?),
    autoplay: true,
    ..Options::default()
};

canvas.set_attribute("aria-label", "Upload progress")?;
let player = Player::new(&runtime, &canvas, &asset, &options)?;
player.ready().await?;
player.set_input(&Name::new("progress")?, Input::Number(50.0))?;
```

Here `canvas` is an attached `HtmlCanvasElement` with a definite CSS width and height. The script is the official runtime served by your app; its Wasm file must come from the same package version. If you use a JavaScript bundler instead, pass its imported namespace to `Runtime::new`. Hold the player in owner-local state for the canvas's lifetime and release it in the Leptos owner's cleanup. It must stay on the browser thread that created it.

`ready()` returns an error for a rejected file, missing scene, external asset, or a load that takes more than 15 seconds. `set_input` returns an error for an unknown name, incorrect type, or non-finite number. `Input::Boolean` sets a boolean, and `Input::Trigger` fires a trigger. These methods support existing files that use Rive's legacy state-machine inputs; they do not expose view-model data binding yet.

## Keep the animation useful

Give the shared element an explicit `aria_label`, or set `aria_hidden(true)` for decoration. A directly owned browser canvas needs the corresponding HTML attributes. Present meaningful status and interactive controls beside the illustration: file-authored pointer and keyboard listeners are disabled. Audio and automatic URL events are disabled too.

Playback is paused by default. The shared element's `running(true)`, or a direct player's `play()`, requests animation. Pausing keeps the current pose. Browser rendering stops offscreen, in a hidden document, or under reduced motion. Native hosts stop callbacks for idle, hidden, inactive, or reduced-motion scenes. Resuming does not fast-forward through time spent in the background. Reduced motion preserves a visible pose.

Cloning an `Asset` shares its bytes. Browser players transfer them once to JavaScript; native players retain parsed scenes and reuse their raster buffer at a stable size. Frames are uploaded directly as premultiplied pixels, without encoding images or putting pixels into UI JSON. Backing stores are limited to 4,194,304 pixels per player, 4,096 pixels per axis, and a device scale of 2. Native surfaces additionally cap players at 16 and divide an 8,388,608-pixel budget among them. Large views render at reduced resolution. Direct native `Player` users own their scheduling and aggregate budgets.

## Choose trusted, self-contained files

Browser files must embed their fonts, images, and audio. Native playback rejects those asset types entirely. The browser player disables Rive's asset CDN and public Wasm fallback and rejects external assets. The crate accepts bytes rather than URLs and never selects a CDN or injects a script. Native builds pin the upstream C++ runtime and require a C++17 compiler and the platform SDK; they download no runtime dependencies during the build.

Files are limited to 8 MiB before copying, with checked version fields in the header. These checks do not authenticate a file, validate its object graph, limit all decoded resources, or sandbox the upstream parser. Load only files you ship or verify yourself, and keep the upstream runtime patched. Native runtime objects stay on their creating UI thread, outside the Wasm interpreter's sandbox. Do not pass arbitrary uploads to either runtime. The browser load deadline cannot interrupt a synchronous parser or script.

Wasm guests have a stricter 64 KiB limit for each node's complete JSON properties, including the base64-encoded file. Compact input updates also count the original scene's property payload against that limit; they do not reset the retained scene's memory allowance. The host caps retained properties across all guest nodes at 4 MiB. Direct native Rust callers use the file limit above.
