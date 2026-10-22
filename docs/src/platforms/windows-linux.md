# Windows and Linux

The shared app runs as Wasm in native desktop runners. Windows uses Win32 controls through `leptoskit_windows`. Linux uses GTK 4 through `leptoskit_gtk`. Both consume the same component tree and Yoga layout.

## Build the shared app

`leptoskit new amazing` generates both runners. On Windows, build the Windows app:

```sh
leptoskit build --platform windows
```

On Linux, build the GTK app:

```sh
leptoskit build --platform linux
```

Windows needs the MSVC Rust toolchain and Windows SDK. Linux needs GTK 4.6 or newer and `pkg-config`. Each app build also needs the Rust `wasm32-unknown-unknown` target for the shared application.

Keep native dependencies and lifecycle changes in `windows/` or `linux/`. Application routes, components, and state stay in `amazing_app`. Build outputs and generated registration code go into `.leptoskit/windows/` or `.leptoskit/linux/`.

To prepare the Rust inputs without producing the final desktop executable, run `leptoskit native prepare windows` or `leptoskit native prepare linux`. See [host projects](host-projects.md) for source ownership, plugins, and code push.

## Platform integration

Both backends implement [`NativeBackend`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_native). They create widgets, update properties and layout, and apply child mutations in batch order. Native font measurement supplies the layout engine's text metrics.

The Windows runner embeds the common-controls application manifest and updates layout when the window's size or DPI changes. The GTK runner hosts the app in the GTK event loop and forwards native control events to the Wasm app. Neither runner embeds a browser.

The backend READMEs describe supported controls and native verification: [Windows](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_windows) and [GTK](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_gtk). Compiling on another OS does not verify native rendering or input. Run the app and its widget tests on the target OS.
