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
- **A surface's own `browsers`** leaves it out of the other packages. The side panel and the developer tools panel are left out of Safari's on their own, because Safari has neither.
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

`leptoskit::extension` also has `storage`, `tabs`, `action` (the toolbar button's badge), `scripting`, `permissions`, `alarms`, `context_menus`, `commands`, `notifications`, `side_panel`, and `devtools`. Each works the same in every browser, returns `Error::Unavailable` where a browser lacks it, and stops listening when its handle is dropped.

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

## A panel in the developer tools

An extension for developers often belongs beside Elements and Console, inspecting the page someone is working on. `[extension.devtools_page]` adds a panel to the developer tools of Chromium browsers and Firefox. It needs no permission:

```toml
[extension.devtools_page]
route = "/devtools"
panel = "/devtools/panel"
```

The developer tools open `route` out of sight each time they open for a tab. That page shows nothing: it adds the panel, which opens `panel` when someone chooses its tab:

```rust,ignore
/// The `/devtools` route: adds the panel, and shows nothing.
fn add_panel() -> Element {
	wasm_bindgen_futures::spawn_local(async {
		let _ = leptoskit::extension::devtools::create_panel("Inspector", "assets/icon-32.png").await;
	});
	View()
}
```

The panel reads the page being inspected with `devtools::eval`, which runs an expression there and returns its value, and follows the page when it reloads or moves on with `devtools::on_navigated`. Safari has no developer tools API, so its package leaves the panel out. [leptoskit DevTools](../guide/26-devtools.md) has a panel built this way.

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

## Safari

Safari installs extensions only from inside an app, so when `browsers` has `safari`, your macOS and iOS apps carry the extension:

```bash
leptoskit build --platform macos
leptoskit build --platform ios
```

Each of those builds first packages the extension for Safari, then:

- **macOS:** adds `Contents/PlugIns/SafariExtension.appex` to the app, signed with the App Sandbox entitlement that Safari requires of extensions.
- **iOS:** adds a `SafariExtension` app extension target to the generated Xcode project, and embeds it in the app.

The extension's bundle identifier is your `native.package_id` plus `.SafariExtension`; set `[extension.safari].bundle_id_suffix` to change the last part. Apple requires it to start with the app's.

People turn the extension on themselves, after installing the app: on a Mac in Safari's settings, under **Extensions**, and on an iPhone or iPad in **Settings**, then **Apps**, **Safari**, **Extensions**. Tell them how from your app's first screen. A development build on a Mac is signed only for this computer, so Safari shows it after you turn on **Allow unsigned extensions** in Safari's **Develop** menu.

Safari has no side panel or developer tools panel, and can't replace the new tab page, so those surfaces aren't in its package.

## Publish

`leptoskit publish` uploads the packages to the Chrome Web Store, Microsoft Edge Add-ons, and addons.mozilla.org, and lists what's left in each store's dashboard. [Publishing to the stores](../guide/24-publishing.md) covers it.

## Develop and test

`leptoskit dev` runs the extension beside your other platforms, in Chrome for Testing, the build of Chrome meant for automation: Chrome 137 and later no longer load an unpacked extension from the command line. Install it once:

```bash
npx @puppeteer/browsers install chrome@stable chromedriver@stable --path ~/.cache/puppeteer
```

Or set `LEPTOSKIT_CHROME`, and `LEPTOSKIT_CHROMEDRIVER`, to the executables you have. `leptoskit doctor` says which one it found.

```bash
leptoskit dev --platform extension
```

That builds a development package, opens Chrome for Testing with it loaded, and rebuilds on every change. The extension reloads itself when the new build is ready; open extension pages close, as they do whenever an extension reloads.

```bash
leptoskit test extension
```

That builds the extension and checks it in Chrome for Testing, through chromedriver:

- every page surface shows something, and the developer tools' hidden page starts;
- the background worker is running;
- each content script puts its frame on a page whose content security policy forbids scripts and WebAssembly, as most large sites' do.

To try a release build by hand, load the unpacked folder: in Chrome, turn on developer mode at `chrome://extensions` and choose **Load unpacked**; in Firefox, open `about:debugging`, choose **This Firefox**, and **Load Temporary Add-on**, picking the folder's `manifest.json`.

## Start a new one

`leptoskit new my_app --extension` scaffolds an app with an extension: a popup at the home screen, a background worker that answers messages, and the Chromium and Firefox packages.

## Plugins

A runtime plugin whose browser side needs an extension permission declares it, and the build adds it to the manifest: the notifications plugin adds `notifications`, and the clipboard plugin adds `clipboardRead` and `clipboardWrite`. A plugin crate declares its own in `[package.metadata.leptoskit.runtime]`, with `extension-permissions = ["…"]`.

## What doesn't work yet

- **Firefox in development:** `leptoskit dev` and `leptoskit test extension` use Chrome for Testing. Load the Firefox package from `about:debugging` to try it.
- **Icons:** the build uses `assets/icon-16.png`, `icon-32.png`, `icon-48.png`, and `icon-128.png` when they exist, and doesn't make them for you.
