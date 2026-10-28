# Rive animations

Use Rive when a status illustration needs to move or respond to your application's state. On the web, `leptoskit_rive` plays an app-owned `.riv` file on a canvas and accepts typed state-machine inputs. A paused player keeps its current pose visible, and dropping it releases its renderer.

This API targets browser Wasm only. It does not add a shared-tree component or native Rive widgets for macOS, iOS, Android, Windows, Linux, or HarmonyOS. Server-rendered pages can include the canvas and alternative content; create the player after the browser has attached the canvas.

## Add a player

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

Give the canvas an accessible label, or set `aria-hidden="true"` for decoration. Present meaningful status and interactive controls in HTML beside it: file-authored pointer and keyboard listeners are disabled. Audio and automatic URL events are disabled too.

Playback is paused by default. `play()` requests animation, while `pause()` keeps the current pose. Rendering stops when the canvas is offscreen, the page is hidden, or the user requests reduced motion. It resumes only when playback is requested and those conditions allow it. Reduced motion preserves the initial pose rather than leaving a blank canvas.

Cloning an `Asset` shares its bytes. Each player transfers them once to JavaScript; animation frames stay in the upstream runtime. The canvas backing store is limited to 4,194,304 pixels, 4,096 pixels per axis, and a device scale of 2. A large CSS canvas therefore renders at reduced resolution.

## Choose trusted, self-contained files

Embed fonts, images, and audio in the `.riv` file. The player disables Rive's asset CDN and public Wasm fallback and rejects external assets. The crate accepts bytes rather than URLs and never selects a CDN or injects a script.

Files are limited to 8 MiB before copying, with checked version fields in the header. These checks do not authenticate a file, validate its object graph, limit decoded resources, or sandbox scripts. Load only files you ship or verify yourself, keep the upstream runtime patched, and do not pass arbitrary uploads to a page's main-thread runtime. The load deadline cannot interrupt a synchronous parser or script.
