# Browser extensions

Many apps want a browser extension next to the website and the phone app: a popup that shows the same account, a side panel that stays open beside whatever page is open, or a button on someone else's site. That usually means a second codebase, in another language, that repeats the app's screens, its state, and its API client.

A leptoskit extension is the app you already have, shown in a few more places. Each surface opens one of your app's routes, and one build packages it for Chromium browsers (Chrome, Edge, Brave, Opera, Arc, Vivaldi), Firefox, and Safari.

## An example: clipping quotes

[Commonplace](https://github.com/ifiokjr/leptoskit/tree/main/examples/commonplace) is a research notebook with a website, native apps, and an extension that clips quotes from the pages you read. Its extension has every surface:

| Surface      | Route                 | What it does                                                                                 |
| ------------ | --------------------- | -------------------------------------------------------------------------------------------- |
| Popup        | `/clip`               | Clip the page you're on, or hand over to the clipper to pick a passage                       |
| Side panel   | `/panel`              | A project's clips beside the page you're reading                                             |
| Options      | `/settings`           | Sign in, choose the project clips go to, and choose the sites that show highlights           |
| New tab      | `/reading`            | Your projects, in every new tab, on Chromium and Firefox                                     |
| `clipper`    | `/overlay/clipper`    | A frame beside the selected text, shown when you press the shortcut or the context menu item |
| `highlights` | `/overlay/highlights` | Highlights what you've clipped on the sites you allowed                                      |
| Background   |                       | The context menu, the shortcut, the badge, and saving clips while offline to send them later |

## Turn it on

Add `extension` to your platforms and describe the extension in `leptoskit.toml`. Every surface except the background names a route, and the build fails if a route doesn't exist:

```toml
[app]
platforms = ["web", "ios", "android", "macos", "extension"]

[extension]
name = "Commonplace"
description = "Clip quotes from anything you read into your Commonplace research projects."
browsers = ["chromium", "firefox", "safari"]
server = "https://commonplace.example.com"
permissions = ["activeTab"]
optional_host_permissions = ["https://*/*"]

[extension.popup]
route = "/clip"

[extension.side_panel]
route = "/panel"

[extension.options]
route = "/settings"

[[extension.content_scripts]]
name = "clipper"
route = "/overlay/clipper"
inject = "on_request"

[extension.background]
events = ["runtime.message", "runtime.installed", "context_menus", "commands"]

[extension.commands.clip]
suggested_key = "Alt+Shift+C"
description = "Clip the selected passage"

[extension.firefox]
id = "commonplace@example.com"
data_collection = ["websiteContent", "personallyIdentifyingInfo"]
```

- **`server`** is where your app's API is. Its origin becomes a host permission, and `leptoskit::extension::runtime::server()` returns it.
- **`permissions`** are added to the ones the extension needs for itself, such as `storage`, `scripting` for content scripts injected at run time, and `sidePanel`.
- **A surface's own `browsers`** leaves it out of the other packages. The side panel is left out of Safari's on its own, because Safari has none.
- **Firefox** needs an add-on ID, which addons.mozilla.org keeps for good, and a declaration of the data the extension sends anywhere. Use `["none"]` when it sends nothing.

Code push can't be on while `extension` is a platform: extension stores forbid code downloaded after their review.

## Write the entry points

Give your app crate an `extension` feature, and name your entry points once:

```toml
[features]
extension = ["csr", "leptoskit/extension"]
```

```rust,ignore
#[cfg(feature = "extension")]
leptoskit::extension::entry!(main: extension::main, background: extension::background);

/// Every surface opens at its configured route, with an in-memory router.
pub fn main(surface: leptoskit::extension::Surface) -> impl Fn() -> Element + Send + Sync + 'static {
	let route = Route::parse(surface.route()).unwrap_or(Route::Library);
	let navigation = Navigation::new(route);
	move || navigation.render()
}

/// The worker without a screen: register what it answers.
pub fn background(worker: &leptoskit::extension::Background) {
	worker.on_message(|message: Message, _sender| async move { answer(message).await });
	worker.on_command(|command| async move { /* … */ });
}
```

An extension build turns on your `web` feature too, so keep your website's own start function out of it with `#[cfg(all(feature = "web", not(feature = "extension")))]`.

Any surface sends the worker a message with `runtime::send`, and the worker's `on_message` handler returns the reply. The message and reply are your own serde types:

```rust,ignore
let reply: Reply = leptoskit::extension::runtime::send(&Message::Clip(input)).await?;
```

`leptoskit::extension` also has `storage`, `tabs`, `action` (the toolbar button's badge), `scripting`, `permissions`, `alarms`, `context_menus`, `commands`, `notifications`, and `side_panel`. Each works the same in every browser, returns `Error::Unavailable` where a browser lacks it, and stops listening when its handle is dropped.

## Content scripts

A content script shows part of your app on another site's page. Most large sites forbid compiling WebAssembly on their pages, so your app doesn't run in the page: it runs in a frame of your extension's own page, in a closed shadow root that the page's styles and scripts can't reach. A small generated script places that frame and answers its questions about the page. Your app talks to it through `leptoskit::extension::page`:

```rust,ignore
let page = leptoskit::extension::page::connect().await?;
let info = page.info().await?; // the address, title, author, site, and publication date
let selection = page.selection().await?; // the selected text, and some text on each side
page.place(Placement::NearSelection { width: 380.0, height: 360.0 }).await?;
page.highlight(&quotes).await?; // marks each quote where it appears
```

A content script reaches pages in one of three ways, set with `inject`:

| `inject`             | Where it runs                                                                             | Permission it needs                              |
| -------------------- | ----------------------------------------------------------------------------------------- | ------------------------------------------------ |
| `manifest` (default) | Every page its `matches` cover                                                            | Host permission for each, granted at install     |
| `on_request`         | The tab you call `scripting::inject` on, right after the user invokes the extension there | None, with `activeTab`                           |
| `registered`         | The sites you pass to `scripting::register`, after the user allows them                   | Each site, asked for with `permissions::request` |

Asking for no host permission at install, as Commonplace does, is what gets an extension through store review fastest.

## Signing in

An extension signs in for a bearer token, the way the native apps do, and keeps it in extension storage, where its pages and its worker share it. [Sign-in](../guide/23-sign-in.md) accepts that from the extension's own origin.

## Build it

```bash
leptoskit build --platform extension --release
```

That builds your client once, with the `extension` feature, and writes one package per browser family:

| Output                                      | What it is                                                                                         |
| ------------------------------------------- | -------------------------------------------------------------------------------------------------- |
| `.leptoskit/extension/chromium/` and `.zip` | For the Chrome Web Store and Microsoft Edge Add-ons                                                |
| `.leptoskit/extension/firefox/` and `.zip`  | For addons.mozilla.org                                                                             |
| `.leptoskit/extension/firefox-source.zip`   | Your project's source and the commands that rebuild the package, which addons.mozilla.org asks for |
| `.leptoskit/extension/safari/`              | Safari's package                                                                                   |

The zips are deterministic: the same source gives the same bytes. Every generated page loads its own script, because extension pages forbid inline scripts, and the pages may load WebAssembly only from the package.

To try it, load the unpacked folder: in Chrome, turn on developer mode at `chrome://extensions` and choose **Load unpacked**; in Firefox, open `about:debugging`, choose **This Firefox**, and **Load Temporary Add-on**, picking the folder's `manifest.json`.

## What doesn't work yet

- **Safari:** the build writes Safari's package, but doesn't yet put it inside your macOS and iOS apps, which is how Safari installs extensions.
- **Development:** `leptoskit dev` doesn't run the extension yet; build it, and reload it in the browser.
- **Publishing:** the zips are ready for the stores, but `leptoskit publish` doesn't upload them yet.
- **Plugins:** the runtime plugins don't yet declare the extension permissions they need, so add those to `permissions` yourself.
- **Icons:** the build uses `assets/icon-16.png`, `icon-32.png`, `icon-48.png`, and `icon-128.png` when they exist, and doesn't make them for you.
