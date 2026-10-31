# Browser extensions

A leptoskit extension is the app's own routes in a few more places. `[extension]` in `leptoskit.toml` lists the surfaces, and each opens one of the app's routes:

| Surface                      | Config                          | Notes                                                    |
| ---------------------------- | ------------------------------- | -------------------------------------------------------- |
| Toolbar popup                | `[extension.popup]`             |                                                          |
| Side panel                   | `[extension.side_panel]`        | Firefox's sidebar; not in Safari                         |
| Options page                 | `[extension.options]`           |                                                          |
| New tab page                 | `[extension.new_tab]`           | not in Safari                                            |
| Frames on other sites' pages | `[[extension.content_scripts]]` | `inject = "manifest"`, `"on_request"`, or `"registered"` |
| Background worker            | `[extension.background]`        | no screen; `events` lists what wakes it                  |

The app crate has an `extension` feature and names its entry points once:

```rust
#[cfg(feature = "extension")]
leptoskit::extension::entry!(main: extension::main, background: extension::background);
```

- `main(surface)` returns the function that renders that surface, usually `Navigation::new(Route::parse(surface.route())…)` and `move || navigation.render()`.
- `background(&worker)` registers handlers: `on_message`, `on_installed`, `on_alarm`, `on_context_menu`, `on_command`, `on_tab_updated`.
- Surfaces talk to the worker with `leptoskit::extension::runtime::send(&message)`; the message and reply are the app's own serde types.
- The website's own `#[wasm_bindgen(start)]` must be off in extension builds: `#[cfg(all(feature = "web", not(feature = "extension")))]`.

## Content scripts

Most large sites forbid compiling WebAssembly in their pages, so a content script's interface runs in a frame of the extension's own page, inside a closed shadow root. The app reaches the page through `leptoskit::extension::page`:

- `connect()`;
- `info()`: the address, title, author, site, and date;
- `selection()`;
- `place(Placement::…)`;
- `highlight(&quotes)`;
- `scroll_to(id)`;
- `close()`;
- `on_event`.

Prefer `inject = "on_request"` with the `activeTab` permission, injected with `scripting::inject(tab, name)` after the user invokes the extension. It needs no host permission, which is what passes store review fastest.

## Commands

```bash
leptoskit build --platform extension [--release]   # .leptoskit/extension/{chromium,firefox,safari}/, zips, firefox-source.zip
leptoskit dev --platform extension                 # Chrome for Testing, reloaded on every change
leptoskit test extension                           # every page, the worker, and each content script in a real browser
```

- `dev` and `test` need Chrome for Testing and chromedriver: `npx @puppeteer/browsers install chrome@stable chromedriver@stable --path ~/.cache/puppeteer`, or `LEPTOSKIT_CHROME` and `LEPTOSKIT_CHROMEDRIVER`.
- Safari's package goes inside the macOS and iOS apps, built with `leptoskit build --platform macos` and `ios`. People turn it on in Safari's settings.
- Extension permissions come from `[extension].permissions`, from what the surfaces need, and from the plugins the app depends on (`extension-permissions` in their metadata).
- Explain each permission in `store/permissions.toml`; `leptoskit publish` pastes those sentences into the store steps.
