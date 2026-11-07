# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-07)

Grouped release for `release`.

### Features

#### Interfaces from agents

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #550](https://github.com/leptoskit/leptoskit/pull/550)

`leptoskit_agent_ui`, also `leptoskit`'s `agent-ui` feature, renders interfaces an AI agent writes as text while your app runs. The agent answers in Markdown with components in it — `<Stat>`, `<Chart>`, `<Slider>`, `<Button>`, and more — declares variables with `<Var>` and formulas with `<Let>`, and writes `{formulas}` in text that follow the user's input without a new message. `<View>`, `<Text>`, and `<Pressable>` let it build layouts the other components don't cover. `AgentUi::view` follows a streaming answer and shows a component that is still open. Button presses reach `on_action` with the user's state, and the app decides what they do.

Documents style with Tailwind classes and colour roles such as `bg-surface` and `text-muted`, never colours. Each role reads your `[theme.colors]` entry of the same name, and its `-dark` twin in dark appearance, so documents and the built-in components match your app in light and dark without the agent knowing either; `agent_ui::Theme` renames roles and adds your own.

Sliders, switches, checkboxes, radios, and spinners are the platform's own controls, updated in place so a slider keeps its drag. `Widgets::WithoutControls` draws them from buttons for macOS, iOS, and Android shells generated before 0.3, and `Widgets::Basic` also leaves out vector paths for HarmonyOS.

Documents are data, never code: formulas run on a fixed budget, classes that would lay content over the app or scroll are refused, images load only from allowed addresses, and `Limits` bound every document. `AgentUi::reference` writes the language reference for the agent from the components and colour roles registered right now, and `AgentUi::check` lists a document's problems by line and column.

#### Motion for the shared component tree

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #554](https://github.com/leptoskit/leptoskit/pull/554)

Elements can move. `motion(Motion)` describes the poses an element moves between, as opacity, offset, scale, rotation, clipping, and blur, and what drives the movement: appearing (`Motion::enter`), scrolling into view (`in_view`), or a named container scrolling past (`timeline` on the container and `follow` on the elements inside it). `stagger` on a container starts its children's motions one after another. Timing comes from a duration and an `Easing`, or from a `Spring`, and a new `key` plays an enter motion again.

The web renders motion as CSS animations and scroll-driven animations, with springs as `linear()` easing, so nothing runs in script. People who ask for less motion, and browsers without scroll timelines, see every element at rest at once. Native platforms show the resting pose too: motion is web-only for now, and native apps never receive it, so adding it changes nothing they build.

The leptoskit.dev landing page is redesigned around it: one component tree becomes each platform's own widgets as you scroll, with every movement written as `Motion` and the hand-written animation script removed.

### Fixes

- **The live example on leptoskit.dev answers sooner.** The landing page's live example loaded the whole hosted app's Wasm client, management console included, before its button worked. The console now loads as a lazy chunk of its own, the client builds with `optimize = "smallest"`, and the page starts the client, its glue, and the Rive runtime downloading together instead of one after another. The example's module drops from 370 KiB to 188 KiB gzip-compressed, and on a phone emulated with Lighthouse's slow 4G and 4x CPU slowdown its first press updates the count about half as long after the example scrolls into view. The hosted console loads the same module and then its chunk, and says the app could not load if the chunk never arrives. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #557](https://github.com/leptoskit/leptoskit/pull/557)
- **Web builds ship the app's own Wasm client.** `leptoskit build` bound the first Wasm module cargo reported. When the client crate depended on another crate that also declares a `cdylib`, cargo compiled that dependency to Wasm first, and the build shipped the dependency's module under the app's name: the page loaded without an error, but the app's start function never ran. Web builds and native guest builds now take the module of the client package itself. This is what kept the live example on leptoskit.dev from responding to presses. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #556](https://github.com/leptoskit/leptoskit/pull/556)
- **Over the air updates get their own section.** The home page and README now lead with one component tree and real native widgets. Over-the-air updates have their own section, describing code push for installed native apps and which generated hosts install releases. _Packages:_ 🟢 _leptoskit_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #552](https://github.com/leptoskit/leptoskit/pull/552)

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-06)

Grouped release for `release`.

### Breaking changes

#### leptoskit_testing_library: browser component tests

_Packages:_ 🔴 _leptoskit_testing_library_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_testing_library` provides Testing Library queries and widget-test actions for leptoskit components mounted in a real browser.

It mounts through the web renderer and shares finders with `leptoskit_testing`, so a team can test the browser build with the same vocabulary it uses for native widget tests.

#### Start the first release at 0.2.0

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #486](https://github.com/leptoskit/leptoskit/pull/486)

This is leptoskit's first public release, and every crate in it is `0.2.0`. The workspace, its internal crate dependencies, and the example apps that pin them all start from `0.1.0`, and a breaking (`major`) change before 1.0 raises the minor version, so the release lands on `0.2.0`. Several of the crate names already have a `0.1.0` on crates.io, which never accepts the same version twice, so the first release has to sit above it.

Apps that `leptoskit new` creates pin the exact version of the CLI that made them, `=0.2.0`.

#### leptoskit_yoga: flexbox and CSS grid layout

_Packages:_ 🔴 _leptoskit_yoga_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_yoga` is a from-scratch layout engine that turns one `Style` into flexbox or CSS grid geometry, with no C dependencies and no platform lock-in.

`leptoskit_native` calls it for every commit; backends only apply the frames it returns. It is also usable on its own when a Rust tool needs React Native-style layout or a CSS grid implementation without a browser.

#### leptoskit_use: cross-platform hooks

_Packages:_ 🔴 _leptoskit_use_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_use` provides cross-platform reactive primitives inspired by leptos-use: state, derived values, watchers, timers, storage, and viewport breakpoints without a DOM or executor.

The hooks read leptos signals and accept host services explicitly, so the same code runs in a browser and in a native guest. The `local-storage` feature adds persistent JSON storage in native app directories and the browser's local storage, and the `web` feature adds browser-only hooks such as media queries and event listeners. It is the utility layer beneath app screens, not a UI framework.

#### leptoskit_native: components, Tailwind, and the renderer

_Packages:_ 🔴 _leptoskit_native_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_native` is the native side of the shared tree: builders for every component, from `View`, `Text`, and `Button` to `TextInput`, `PasswordInput`, `Slider`, and `ScrollView`, the Tailwind compiler, the retained element tree, and the diff that turns a commit into a mutation batch.

Every native backend implements `NativeBackend` and consumes the mutations this crate produces; the web build uses its optional `web` feature to render the same builders through leptos. App code normally reaches it through `leptoskit::prelude`, not as a direct dependency, so one screen works on every target.

#### leptoskit_testing: native widget tests

_Packages:_ 🔴 _leptoskit_testing_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_testing` runs Flutter-style widget tests over the production renderer and Yoga layout, so a screen can be tested without a device or simulator.

`WidgetTester` renders a screen, finds elements by text, label, role, or test id with Testing Library-style queries (`get_by_text`, `query_by_label`, and the rest), acts on them, and pumps the renderer a bounded number of times while waiting for a result. The `native-ui` feature goes further and drives the real app through each platform's own automation: XCUITest on iOS, UI Automator on Android, XCTest on macOS, UI Automation on Windows, and AT-SPI on Linux.

#### leptoskit_tui: the terminal backend

_Packages:_ 🔴 _leptoskit_tui_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_tui` draws the same shared element tree in the terminal, for long-lived interactive apps such as editors, dashboards, and chat clients, with keyboard focus, the cursor, mouse input, and scrolling handled by the backend.

Every cell is 8×16 logical pixels, so Yoga layout, Tailwind spacing, and text measurement land on the monospace grid. `leptoskit dev --platform tui` and `leptoskit build --platform tui` run it, and `examples/opencode` is a full coding-agent interface built with it. The terminal's font is the user's, so font sizes and weights become attributes and colors; `Image` and `Vector` draw a placeholder, Rive playback is unsupported, and right-to-left text is not reordered.

#### leptoskit_appkit: the macOS backend

_Packages:_ 🔴 _leptoskit_appkit_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_appkit` is the macOS backend: it applies leptoskit mutation batches to real `NSView`s, from labels and buttons to `NSSecureTextField` password fields, sliders, and scroll views, and measures text with AppKit's own fonts.

A `NativeSurface` built on `AppKitBackend` renders the same tree the web build renders as DOM, and `snapshot_png` draws it offscreen without a window. AppKit is the reference backend that the others follow, and it draws every built-in component. The `macos` feature brings it in, but the macOS app `leptoskit new` generates still draws through a smaller Swift renderer, which covers ten components so far. On other platforms the crate compiles to an empty library.

#### leptoskit_uikit: the iOS backend

_Packages:_ 🔴 _leptoskit_uikit_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_uikit` is the iOS backend: it renders shared leptoskit components as real UIKit views and controls, such as `UISwitch`, `UISlider`, `UIProgressView`, and secure text fields for password inputs, and measures text with Core Text.

It draws every component when you host screens with `UIKitHost` in your own app, which keeps the layout inside a view controller and reports rotation and safe-area insets. The app generated by `leptoskit build` does not use it yet: that app draws ten components through its own Swift renderer. iOS 15 is the minimum, and images load from local files and app bundle resources, not web URLs.

#### leptoskit_android: the Android backend

_Packages:_ 🔴 _leptoskit_android_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_android` is the Android Views backend: it applies the shared mutation stream to real Android views, with Yoga owning exact outer frames and Android owning each control's interior.

Compile `android/WidgetHost.kt` into an Android app and construct `AndroidBackend::new` on the UI thread. Every built-in primitive renders a real view; callbacks propose typed events that the app commits through `NativeSurface`.

#### leptoskit_gtk: the Linux backend

_Packages:_ 🔴 _leptoskit_gtk_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_gtk` renders the shared element tree as real GTK 4 Linux widgets, consuming the same Yoga mutation stream as every other backend, and draws every built-in component. It needs GTK 4.6 or newer.

The Linux app `leptoskit new` generates uses it through the `linux` feature. A host initializes GTK on the application thread, hands a `GtkBackend` to `NativeSurface`, shows a committed root with `open_window`, and runs the loop with `run_application`; GTK objects stay on their creating thread.

#### leptoskit_windows: the Windows backend

_Packages:_ 🔴 _leptoskit_windows_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_windows` renders shared leptoskit elements as Win32 windows and native controls: every element owns a child window, and interactive elements, password fields included, use real Windows controls so keyboard and accessibility behave natively.

`leptoskit build --platform windows` produces an app that draws every component with it and embeds the common-controls manifest for you; a hand-written host must embed the crate's `app.manifest` itself. The backend stays on one UI thread, reuses the shared Yoga layout, and never embeds a browser. `Switch` is drawn as a checkbox, rounded corners have unsmoothed edges, and the generated app can't host plugins yet.

#### leptoskit_aria: accessible primitives

_Packages:_ 🔴 _leptoskit_aria_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_aria` provides headless accessible components for the shared element tree: disclosure, toggle buttons, tabs, labeled fields, and dialogs with roles and state wired for web and native.

It owns behavior, not pixels: app code styles the pieces with Tailwind and gets consistent focus, keyboard, and ARIA semantics on every backend. `leptoskit_form` builds its fields on top of it.

#### leptoskit_analytics: see which routes fail or slow down

_Packages:_ 🔴 _leptoskit_analytics_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #513](https://github.com/leptoskit/leptoskit/pull/513)

`leptoskit_analytics` keeps bounded, in-memory request and browser measurements for a leptoskit app, so you can see which routes fail or slow down without running a separate observability service.

Enable `analytics` alongside `server` on the `leptoskit` dependency. The `server` feature instruments Axum, continues W3C `traceparent` correlation, and serves a protected JSON evidence API at `/_leptoskit/analytics` plus a same-origin browser module at `/_leptoskit/analytics.mjs`. The browser module honors Global Privacy Control and Do Not Track, and collects no user or session ID, cookie, query string, or page content. Each replica reports its own snapshot; connect the deployment service (`lk console serve`) for durable fleet history, or export OTLP/HTTP metrics.

#### leptoskit_deploy_console: self-hosted deployments

_Packages:_ 🔴 _leptoskit_deploy_console_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #513](https://github.com/leptoskit/leptoskit/pull/513)

`leptoskit_deploy_console` is the optional service behind `lk console`: one process that hosts the deployment dashboard, a durable operation queue, a scoped API for agents, and analytics history, backed by SQLite.

Run `lk console serve` on a trusted worker with Docker, OpenTofu, Kamal 2, Git, and SSH, then `lk console connect` from the app's checkout, and `lk infra plan`, `lk infra apply`, and `lk deploy` run through it. It reuses the CLI's OpenTofu and release engine, binds each apply to a reviewed plan, and keeps interrupted operations for explicit reconciliation. The `server` feature enables the service; the dashboard is a leptoskit component tree rendered on the server. It serves one owner's repositories and is not a multi-tenant executor.

#### leptoskit_form: headless form state

_Packages:_ 🔴 _leptoskit_form_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_form` is headless form state in the style of TanStack Form: typed fields, sync validation, touch and submit tracking, and inputs wired through the aria fields.

A form declares `FieldSpec`s once; the same state drives web inputs and native controls, and `leptoskit_aria` supplies the accessible field wiring. Validation and submission logic stay out of the view layer.

#### leptoskit_config: project configuration

_Packages:_ 🔴 _leptoskit_config_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_config` loads and validates `leptoskit.toml`: app identity and platforms, server and build settings, native targets and plugin grants, code push channels, routes, translations, the database, sign-in, infrastructure, browser extensions, store publishing, plugins, formatting, and lint settings.

The CLI, build pipeline, dev server, and native hosts all read the same typed configuration, so they agree on what the app is. Unknown keys fail at load time instead of being ignored. The schema is versioned (version 0, unstable until version 1); `leptoskit migrate` upgrades an older file, and the `schema` feature generates its JSON Schema.

#### leptoskit_plugin: build-time plugins

_Packages:_ 🔴 _leptoskit_plugin_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin` is the build-hook system shared by leptoskit plugins and leptos plugins: generated code, feature flags, environment variables, asset directories, post-build reports, and a plugin's own `plugin:<name>` commands.

Plugins are declared in `leptoskit.toml`. A Rust plugin crate implements `LeptoskitPlugin`; a separate program in any language answers each hook over a versioned JSON protocol; and leptos plugins are forwarded to the app build as cargo features. Runtime native plugins use the bridge instead; this is the build-time half.

#### leptoskit_bridge: the plugin transport

_Packages:_ 🔴 _leptoskit_bridge_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_bridge` is the bounded binary transport between a Wasm guest and the native plugins it is allowed to call.

The host registers each plugin and grants its methods, so an over-the-air bundle cannot add native capabilities. Every call is bounded in size and number, is cancelled when the guest drops it, and fails with error codes the Swift, Kotlin, and C++ hosts share. Most apps never name it directly; each runtime plugin wraps it in a typed `guest` module.

#### leptoskit_bridge_ffi: the native half of the app host

_Packages:_ 🔴 _leptoskit_bridge_ffi_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_bridge_ffi` is the C ABI, plus the Swift, Kotlin, and C++ code, that lets a native app shell host a leptoskit Wasm guest: it runs the guest, forwards its plugin calls, and draws its tree with the platform's own views.

The macOS, iOS, and Android apps `leptoskit new` generates link it, and HarmonyOS links its experimental N-API layer; app authors never add it themselves. Its Swift and Kotlin renderers draw ten components so far, including password fields, and stop on any other with an "unsupported widget" error. The generated Linux and Windows apps use the GTK and Win32 backend crates instead.

#### leptoskit_build: one app, platform modules

_Packages:_ 🔴 _leptoskit_build_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_build` is the build-script helper behind `platform_module!`: it records which platform-specific modules exist and cfg-selects them while one app keeps a single module tree.

Generated apps call it from `build.rs`. It lets a shared module be replaced by a `.macos.rs`, `.ios.rs`, or `.web.rs` file without changing the imports in the rest of the app.

#### leptoskit_plugin_crypto: portable digests and randomness

_Packages:_ 🔴 _leptoskit_plugin_crypto_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_crypto` is the leptoskit plugin for portable SHA-256, SHA-512, secure random bytes, and UUID v4 with a binary native bridge.

Add `leptoskit_plugin_crypto` as a direct dependency and select the `crypto` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic and hashing and entropy run on the host's Rust worker. Guests enable the `guest` feature and call the host through `guest::Crypto`. Hashing works directly on every target; browser apps enable the `web` feature for random bytes and UUIDs from `Crypto.getRandomValues`. There is no pseudo-random fallback, and encryption and key storage are out of scope.

#### leptoskit_plugin_os: host platform identity

_Packages:_ 🔴 _leptoskit_plugin_os_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_os` is the leptoskit plugin for the host operating system and CPU architecture as seen from inside a Wasm guest.

Add `leptoskit_plugin_os` as a direct dependency and select the `os` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. The values come from compile-time constants of the host build, so the guest sees the real platform even though its own target is wasm32.

#### leptoskit_plugin_clipboard: text clipboard access

_Packages:_ 🔴 _leptoskit_plugin_clipboard_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_clipboard` is the leptoskit plugin for system text clipboard access - read, write, and clear - on iOS, Android, macOS, Windows, Linux, and in the browser.

Add `leptoskit_plugin_clipboard` as a direct dependency and select the `clipboard` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Clipboard` rather than resolving the plugin on every call. iOS, Android, and macOS use the bundled Swift and Kotlin handlers, or experimental pure-Rust ones when `native.experimental-rust-plugins = true`; Rust desktop hosts keep a `Clipboard` alive over the system clipboard, and browser apps call the `web` module. Only plain text is supported; browser operations need a secure context and may need a user gesture.

#### leptoskit_plugin_haptics: haptic feedback

_Packages:_ 🔴 _leptoskit_plugin_haptics_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_haptics` is the leptoskit plugin for selection, impact, and notification feedback on native actuators.

Add `leptoskit_plugin_haptics` as a direct dependency and select the `haptics` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Haptics` rather than resolving the plugin on every call. iOS uses UIKit feedback generators, Android uses platform haptic feedback, and macOS approximates the mobile styles with its trackpad performer, through the bundled Swift and Kotlin handlers or, with `native.experimental-rust-plugins = true`, experimental pure-Rust ones. Browsers use the Vibration API where allowed, and Windows and Linux return `Unavailable`. Success means the OS accepted the request, not that the device vibrated.

#### leptoskit_plugin_battery: battery and power state

_Packages:_ 🔴 _leptoskit_plugin_battery_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_battery` is the leptoskit plugin for battery charge, charging state, and system power-saving snapshots with cancellable change observation.

Add `leptoskit_plugin_battery` as a direct dependency and select the `battery` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Battery` rather than resolving the plugin on every call. Unknown readings stay unknown instead of being reported as a full battery, and a simulator is never presented as physical hardware.

#### leptoskit_plugin_device: device information

_Packages:_ 🔴 _leptoskit_plugin_device_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_device` is the leptoskit plugin for device and operating system information without persistent identifiers or user-assigned names.

Add `leptoskit_plugin_device` as a direct dependency and select the `device` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Device` rather than resolving the plugin on every call. Native guests, Rust desktop hosts, and browsers each return a validated binary snapshot with the fields that platform can actually provide.

#### leptoskit_plugin_secure_store: credential-store secrets

_Packages:_ 🔴 _leptoskit_plugin_secure_store_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_secure_store` is the leptoskit plugin for small binary secrets, such as a refresh token, kept in the platform credential store and scoped to one application.

Add `leptoskit_plugin_secure_store` as a direct dependency and select the `secure-store` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::SecureStore` to get, set, and delete secrets of up to 2048 bytes; all of an app's secrets share one encrypted record limited to 2560 bytes and 64 entries, which hosts can lower. iOS and macOS use the Keychain and Android encrypts with an Android Keystore key, while desktop Rust hosts construct `SecureStore` over the Keychain, Windows Credential Manager, or Secret Service. The browser has no secure store, and there is no key listing, biometric prompt, or cloud sync.

#### leptoskit_plugin_linking: external links and settings

_Packages:_ 🔴 _leptoskit_plugin_linking_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_linking` is the leptoskit plugin for validated external HTTP(S), email, phone, and SMS links plus application settings, without executing shell text.

Add `leptoskit_plugin_linking` as a direct dependency and select the `linking` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Linking` rather than resolving the plugin on every call. Incoming deep links need host lifecycle integration and are not handled by this crate.

#### leptoskit_plugin_passkeys: passkeys and Sign in with Apple

_Packages:_ 🔴 _leptoskit_plugin_passkeys_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_passkeys` is the leptoskit plugin for passkeys and Sign in with Apple through the platform's own credential sheet. The sign-in client in `leptoskit_auth` uses it, so apps rarely call it directly.

Add `leptoskit_plugin_passkeys` as a direct dependency and select the `passkeys` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Passkeys` rather than resolving the plugin on every call. Apple platforms use `ASAuthorization` and Android uses Credential Manager, with WebAuthn JSON crossing the bridge unchanged; Sign in with Apple is Apple-only. Windows and Linux report the calls unavailable, and the sign-in client falls back to the system browser there. The relying party's site must list the app in `apple-app-site-association` and `assetlinks.json`, which `leptoskit::server::app_associations` serves from `[native]`.

#### leptoskit_plugin_web_auth: system-browser sign-in

_Packages:_ 🔴 _leptoskit_plugin_web_auth_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_web_auth` is the leptoskit plugin for sign-in sessions in the system browser - ASWebAuthenticationSession on Apple platforms, Auth Tab on Android, and a loopback redirect on desktop.

Add `leptoskit_plugin_web_auth` as a direct dependency and select the `web-auth` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::WebAuth` rather than resolving the plugin on every call. It follows RFC 8252: the provider page never loads in a web view inside the app.

#### leptoskit_plugin_filesystem: application files

_Packages:_ 🔴 _leptoskit_plugin_filesystem_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_filesystem` is the leptoskit plugin for application-private binary files on native and OPFS in the browser, with bounded messages.

Add `leptoskit_plugin_filesystem` as a direct dependency and select the `filesystem` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Filesystem` rather than resolving the plugin on every call. Every native path stays inside the application-owned data directory; guests cannot select an arbitrary filesystem root.

#### leptoskit_plugin_network: network state

_Packages:_ 🔴 _leptoskit_plugin_network_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_network` is the leptoskit plugin for network state and demand-driven change observation.

Add `leptoskit_plugin_network` as a direct dependency and select the `network` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Network` rather than resolving the plugin on every call. Apple uses `NWPathMonitor`, Android uses `ConnectivityManager`, desktop reports interface state, and the browser uses `navigator.onLine` plus `NetworkInformation`; an active connection never claims a server is reachable.

#### leptoskit_plugin_posture: foldable posture

_Packages:_ 🔴 _leptoskit_plugin_posture_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_posture` is the leptoskit plugin for fold state, hinge geometry, and hinge angles, so a layout can put content on either side of a fold.

Add `leptoskit_plugin_posture` as a direct dependency and select the `posture` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::DevicePosture` rather than resolving the plugin on every call, and browser apps use the `web` module. iOS 27.1 uses the hinge interaction API, Android uses Jetpack WindowManager and the hinge angle sensor, HarmonyOS (experimental) uses its display fold status, and Chromium browsers report the fold without angles. macOS, Windows, and Linux report the plugin unavailable.

#### leptoskit_plugin_dialog: native dialogs

_Packages:_ 🔴 _leptoskit_plugin_dialog_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_dialog` is the leptoskit library for asynchronous native file, directory, save, and message dialogs on macOS, Windows, and Linux, plus browser file selection.

Rust desktop hosts and browser apps call it directly: it has no `native.plugins` entry, no guest API, and no automatic registration, because each dialog needs a running event loop and a live parent window. On desktop, `desktop::FileDialog` and `desktop::MessageDialog` borrow that parent window until the dialog closes, and Linux needs an XDG desktop portal and Zenity. In the browser, `web::open_files` uses the File System Access API from a user action on HTTPS, and `web::read_file` reads a selection under an explicit byte limit; browser save, directory, and message dialogs, and mobile dialogs, are not provided.

#### leptoskit_plugin_document_picker: document import

_Packages:_ 🔴 _leptoskit_plugin_document_picker_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_document_picker` is the leptoskit plugin for bounded document import through the iOS document picker, macOS open panel, Android Storage Access Framework, or an HTML file input.

Add `leptoskit_plugin_document_picker` as a direct dependency and select the `document-picker` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and call `pick`, browser apps call `web::pick` from a user action, and Rust desktop hosts on macOS, Windows, and Linux can call `desktop::pick` with a parent window. The user picks one document; the app receives its name, its MIME type when known, and owned bytes up to 4 MiB, and never gets a reusable path into the document provider.

#### leptoskit_plugin_screen_orientation: orientation requests

_Packages:_ 🔴 _leptoskit_plugin_screen_orientation_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_screen_orientation` is the leptoskit plugin for scoped activity, scene, and browser-document orientation requests with explicit restoration.

Add `leptoskit_plugin_screen_orientation` as a direct dependency and select the `screen-orientation` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::ScreenOrientation` rather than resolving the plugin on every call. iOS and Android hold the policy for the app's own scene or activity and restore the previous one when the hold is dropped, browser apps use `web::hold`, and macOS, Windows, and Linux have no native orientation lock. A pending hold is not an acknowledgement that the OS rotated; the snapshot reports the presented interface, not accelerometer orientation.

#### leptoskit_plugin_light_sensor: ambient light

_Packages:_ 🔴 _leptoskit_plugin_light_sensor_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_light_sensor` is the leptoskit plugin for ambient illuminance in lux from Android's light sensor or the browser's AmbientLightSensor.

Add `leptoskit_plugin_light_sensor` as a direct dependency and select the `light_sensor` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::LightSensor` rather than resolving the plugin on every call. iOS, macOS, and desktop have no supported public ambient-light API and return `Unavailable`; display brightness is never substituted for illumination.

#### leptoskit_plugin_pedometer: step counts

_Packages:_ 🔴 _leptoskit_plugin_pedometer_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_pedometer` is the leptoskit plugin for bounded, demand-driven step-counter readings with explicit platform count domains.

Add `leptoskit_plugin_pedometer` as a direct dependency and select the `pedometer` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Pedometer` rather than resolving the plugin on every call. Only iOS (CoreMotion) and Android (its step counter) count steps; the app supplies the motion usage description or activity-recognition permission. iOS reports steps for the requested interval and can read history from the past seven days, while Android reports its cumulative count since boot, so neither is a daily total. macOS, Windows, Linux, and the browser report no sensor and never invent counts.

#### leptoskit_plugin_accelerometer: accelerometer readings

_Packages:_ 🔴 _leptoskit_plugin_accelerometer_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_accelerometer` is the leptoskit plugin for cancellable acceleration readings including gravity, in metres per second squared, along native device axes.

Add `leptoskit_plugin_accelerometer` as a direct dependency and select the `accelerometer` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Accelerometer` rather than resolving the plugin on every call. iOS and Android use real system sensors, desktop has no sensor, and the browser uses Generic Sensor or DeviceMotion.

#### leptoskit_plugin_application: app identity

_Packages:_ 🔴 _leptoskit_plugin_application_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_application` is the leptoskit plugin for installed host application identity: ID, localized display name, release version, and build number.

Add `leptoskit_plugin_application` as a direct dependency and select the `application` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Application` rather than resolving the plugin on every call. Apple reads `Bundle.main`, Android reads the running package record, and a Rust desktop host supplies its own packaging metadata; browsers have no equivalent installed-app identity.

#### leptoskit_plugin_autostart: login startup

_Packages:_ 🔴 _leptoskit_plugin_autostart_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_autostart` is the leptoskit plugin for explicit per-user desktop login registration with a validated identity and command: a LaunchAgent on macOS, an XDG autostart entry on Linux, and the current user's Run key on Windows.

The native host constructs `Autostart` itself with the app's identity, executable, and arguments; it has no `native.plugins` entry and installs nothing by default. The host calls `enable`, `disable`, or `is_registered` directly, or registers it on the bridge so a guest with the `guest` feature can use `guest::AutostartClient` without choosing the command. Constructing it never enables login startup, an existing entry it doesn't own is never overwritten, and iOS, Android, and the browser return `Unavailable`.

#### leptoskit_plugin_barometer: barometric pressure

_Packages:_ 🔴 _leptoskit_plugin_barometer_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_barometer` is the leptoskit plugin for cancellable atmospheric pressure readings in hectopascals.

Add `leptoskit_plugin_barometer` as a direct dependency and select the `barometer` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Barometer` rather than resolving the plugin on every call. iOS and Android use real pressure sensors (iOS needs an `NSMotionUsageDescription`); desktop and the browser report the sensor as unavailable, and no relative altitude estimate is fabricated.

#### leptoskit_plugin_brightness: screen brightness

_Packages:_ 🔴 _leptoskit_plugin_brightness_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_brightness` is the leptoskit plugin for scoped mobile screen brightness overrides with explicit restoration.

Add `leptoskit_plugin_brightness` as a direct dependency and select the `brightness` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Brightness` rather than resolving the plugin on every call. Android changes only the activity window, iOS changes the screen and restores the previous value, and desktop and the browser return `Unavailable`.

#### leptoskit_plugin_gyroscope: gyroscope readings

_Packages:_ 🔴 _leptoskit_plugin_gyroscope_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_gyroscope` is the leptoskit plugin for cancellable angular velocity readings in radians per second around native device axes.

Add `leptoskit_plugin_gyroscope` as a direct dependency and select the `gyroscope` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Gyroscope` rather than resolving the plugin on every call. iOS and Android use real system sensors, desktop has no sensor, and the browser converts DeviceMotion degrees to radians.

#### leptoskit_plugin_http: bounded HTTP

_Packages:_ 🔴 _leptoskit_plugin_http_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_http` is the leptoskit plugin for scoped asynchronous HTTP clients with bounded request and response bodies, a total timeout, and cancellation by dropping the request.

Add `leptoskit_plugin_http` as a direct dependency, select the `http` plugin in `native.plugins`, and list the origins the app may reach under `[native.plugin-options.http]`; there is no wildcard, and the guest can't widen the list. Guests use `default-features = false, features = ["guest"]` and send requests through `guest::HttpClient`. Native requests use reqwest with rustls on the host's async runtime; browser apps keep the default `browser` feature and use fetch with incremental body reading. Redirects are not followed: native callers get the 3xx response, and the browser rejects it.

#### leptoskit_plugin_keep_awake: display wake locks

_Packages:_ 🔴 _leptoskit_plugin_keep_awake_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_keep_awake` is the leptoskit plugin for scoped display wake locks with cancellation-safe ownership.

Add `leptoskit_plugin_keep_awake` as a direct dependency and select the `keep-awake` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::KeepAwake` rather than resolving the plugin on every call. iOS preserves and restores the idle timer, Android uses the activity's keep-screen-on flag, desktop uses OS assertions, and the browser uses the Screen Wake Lock API; none of them promise background execution.

#### leptoskit_plugin_localization: locale preferences

_Packages:_ 🔴 _leptoskit_plugin_localization_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_localization` is the leptoskit plugin for ordered preferred locales, language and region codes, calendar, timezone, first weekday, and clock preference.

Add `leptoskit_plugin_localization` as a direct dependency and select the `localization` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Localization` rather than resolving the plugin on every call. iOS, macOS, and Android read the system settings, desktop Rust hosts register `desktop::Localization`, and the browser calls `web::snapshot()`. Each snapshot reflects current settings and no global locale is mutated; there are no change events yet, so re-query after the app resumes.

#### leptoskit_plugin_magnetometer: magnetometer readings

_Packages:_ 🔴 _leptoskit_plugin_magnetometer_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_magnetometer` is the leptoskit plugin for cancellable magnetic-field readings in microteslas along native device axes.

Add `leptoskit_plugin_magnetometer` as a direct dependency and select the `magnetometer` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Magnetometer` rather than resolving the plugin on every call. This is a magnetic sensor measurement, not a compass heading, and platform calibration differs. iOS and Android use real system sensors (iOS needs `NSMotionUsageDescription`), the browser needs Generic Sensor support, and desktop has no sensor.

#### leptoskit_plugin_permissions: permissions

_Packages:_ 🔴 _leptoskit_plugin_permissions_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_permissions` is the leptoskit plugin for explicit camera and microphone permission queries and requests.

Add `leptoskit_plugin_permissions` as a direct dependency and select the `permissions` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Permissions` rather than resolving the plugin on every call. iOS, macOS, and Android use the system permission dialogs and browser apps call the `web` module; Windows and Linux have no backend. Querying status never shows a prompt, so request only after the user chooses a feature. Other permissions are not covered here: the location and notifications plugins request their own.

#### leptoskit_plugin_single_instance: single-instance locks

_Packages:_ 🔴 _leptoskit_plugin_single_instance_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_single_instance` is the native-host plugin for explicit process-lifetime instance locks.

Native hosts construct it explicitly; it is not a guest plugin and installs nothing by default. Call `InstanceGuard::acquire` before constructing the app and keep the guard alive; macOS, Linux, and Windows release the OS lock on drop or process exit, including a crash. It only provides mutual exclusion: a second launch gets `None` and does not forward its arguments or focus the first instance's window.

#### leptoskit_plugin_menu: native menus

_Packages:_ 🔴 _leptoskit_plugin_menu_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_menu` is the native-host plugin for bounded native menu definitions with explicit desktop ownership: commands, checkboxes, separators, submenus, and accelerators such as `CmdOrCtrl+S`.

Native hosts construct it explicitly on their UI thread; it is not a guest plugin and installs nothing by default. macOS attaches it as the application menu and Windows attaches it to a window the host owns; Linux has no standalone menu bar, so there it only feeds tray menus. Definitions are validated before any native allocation, and iOS, Android, and the browser only validate definitions and show no menu.

#### leptoskit_plugin_tray: tray icons

_Packages:_ 🔴 _leptoskit_plugin_tray_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_tray` is the native-host plugin for explicitly owned native desktop tray icons with bounded events.

Native hosts construct it explicitly; it is not a guest plugin and installs nothing by default. The host calls `TrayHost::install` after its event loop starts and owns removal; Linux uses the GTK-free KSNI backend, and mobile and the browser have no tray.

#### leptoskit_plugin_speech: text-to-speech

_Packages:_ 🔴 _leptoskit_plugin_speech_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_speech` is the leptoskit plugin for bounded, cancellable text-to-speech with one active utterance per plugin.

Add `leptoskit_plugin_speech` as a direct dependency and select the `speech` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Speech` rather than resolving the plugin on every call. iOS and macOS use `AVSpeechSynthesizer`, Android uses `TextToSpeech`, and browser apps use `web::Speech` over `speechSynthesis`; Windows and Linux have no speech backend yet. Overlapping speech returns `Busy`, and pause, resume, and word-boundary events are not implemented.

#### leptoskit_plugin_sqlite: private SQLite

_Packages:_ 🔴 _leptoskit_plugin_sqlite_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_sqlite` is the leptoskit plugin for bounded, application-private SQLite databases with typed bindings and the same bundled engine on every native platform.

Add `leptoskit_plugin_sqlite` as a direct dependency and select the `sqlite` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Sqlite` for parameterized writes, read-only queries, and atomic transaction batches. The database runs in Rust on the host's worker, so iOS, Android, and macOS behave the same; Windows and Linux hosts that register plugins themselves get the same engine, and the browser has no SQLite backend. Guests cannot choose file paths, attach databases, or run pragmas, and each database is capped at 64 MiB by default, with limits on statement, result, and transaction sizes.

#### leptoskit_plugin_store: JSON settings

_Packages:_ 🔴 _leptoskit_plugin_store_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_store` is the leptoskit plugin for bounded JSON application settings with explicit persistence and atomic ordered batches.

Add `leptoskit_plugin_store` as a direct dependency and select the `store` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Store` rather than resolving the plugin on every call. Each application gets one store and guests cannot choose filesystem paths: native apps save to a file in the app data directory and browser apps use `web::Store` over `localStorage`. Nothing saves until `save` or a committed batch, and the store is not encrypted, so keep credentials in the secure store.

#### leptoskit_plugin_image_manipulator: image transforms

_Packages:_ 🔴 _leptoskit_plugin_image_manipulator_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_image_manipulator` is the leptoskit plugin for bounded JPEG and PNG decode, resize, crop, rotate, flip, and encode using one Rust implementation.

Add `leptoskit_plugin_image_manipulator` as a direct dependency and select the `image_manipulator` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests use `default-features = false, features = ["guest"]` and call `guest::ImageManipulator`, which skips the image codecs; native calls run on the Rust worker so a batch takes one bridge round trip. Browser calls use the same engine compiled to Wasm, and large jobs belong in a Web Worker. Images are limited to 4 MiB, 8,192 pixels per side, and 16 operations per request.

#### leptoskit_plugin_screen_capture: capture protection

_Packages:_ 🔴 _leptoskit_plugin_screen_capture_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_screen_capture` is the leptoskit plugin for window-scoped capture prevention and capture observation as separate capabilities.

Add `leptoskit_plugin_screen_capture` as a direct dependency and select the `screen-capture` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::ScreenCapture` rather than resolving the plugin on every call. The plugin never captures pixels or reads screenshots; Android can protect its owned window and, on Android 14 and later, report screenshots; iOS can observe scene capture and app screenshots without prevention; macOS, Windows, Linux, and the browser report both unavailable.

#### leptoskit_plugin_store_review: store review requests

_Packages:_ 🔴 _leptoskit_plugin_store_review_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_store_review` is the leptoskit plugin for owned-window in-app store review requests through StoreKit and Google Play.

Add `leptoskit_plugin_store_review` as a direct dependency and select the `store-review` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::StoreReview` rather than resolving the plugin on every call. Acceptance never proves a dialog appeared or a review was submitted, and the plugin never substitutes another application's scene.

#### leptoskit_plugin_sharing: native sharing

_Packages:_ 🔴 _leptoskit_plugin_sharing_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_sharing` is the leptoskit plugin for bounded text and HTTP(S) URL sharing through the user-selected native share UI.

Add `leptoskit_plugin_sharing` as a direct dependency and select the `sharing` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and call `share`, and browser apps call `web::share` from a user action. iOS, macOS, Android, and browsers with Web Share are supported; Windows and Linux have no native share sheet yet, and files are not shared. The app supplies the content, the user picks the destination, and completion only means the platform finished its API callback.

#### leptoskit_plugin_tracking_transparency: tracking consent

_Packages:_ 🔴 _leptoskit_plugin_tracking_transparency_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_tracking_transparency` is the leptoskit plugin for explicit iOS App Tracking Transparency status and authorization requests.

Add `leptoskit_plugin_tracking_transparency` as a direct dependency and select the `tracking-transparency` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::TrackingTransparency` rather than resolving the plugin on every call. It never reads advertising identifiers or starts tracking; iOS returns the system status, and other platforms return `Unavailable`.

#### leptoskit_plugin_websocket: WebSocket connections

_Packages:_ 🔴 _leptoskit_plugin_websocket_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_websocket` is the leptoskit plugin for scoped WebSocket connections with bounded messages and explicit lifecycle.

Add `leptoskit_plugin_websocket` as a direct dependency and select the `websocket` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Native connections use tokio-tungstenite with rustls and need a Tokio runtime; browser connections own their socket and event callbacks. Every destination origin is explicit.

#### leptoskit_plugin_arguments: launch arguments

_Packages:_ 🔴 _leptoskit_plugin_arguments_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_arguments` is the leptoskit plugin for immutable launch-argument snapshots with typed, non-exiting clap parsing.

Add `leptoskit_plugin_arguments` as a direct dependency and expose the plugin explicitly from the native host; it has no `native.plugins` entry. Guests enable the `guest` feature and reuse `guest::ArgumentsClient` rather than resolving the plugin on every call. The host chooses which arguments to disclose; mobile and browser process capture returns `Unavailable`, while explicit snapshots and parsing still work.

#### leptoskit_plugin_shell: child processes

_Packages:_ 🔴 _leptoskit_plugin_shell_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_shell` is the leptoskit plugin for scoped, bounded, cancellable desktop child processes built from host-defined command capabilities, such as running a bundled tool with `--version`.

Add `leptoskit_plugin_shell` as a direct dependency, select the `shell` plugin in `native.plugins`, and declare each command under `[native.plugin-options.shell.commands]`: an absolute executable and working directory, the allowed argument slots, an explicit environment, output limits, and a deadline. Guests enable the `guest` feature and call `guest::ShellClient::execute` with a command name and arguments; they never send a shell command string. Commands run on macOS, Linux, and Windows, and cancellation, timeouts, and shutdown clean up the child's process group or Windows Job Object. The generated Windows and Linux apps don't register runtime plugins yet, so there a host constructs `Shell` itself; mobile and the browser have no process backend.

#### leptoskit_plugin_logging: structured logging

_Packages:_ 🔴 _leptoskit_plugin_logging_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_logging` is the leptoskit plugin for structured, bounded application logging: records with a severity, a target, a message, and named fields, written as JSON lines to rotated files.

Add `leptoskit_plugin_logging` as a direct dependency and select the `logging` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and write through `guest::Logger`. Native apps write to `<app data>/logs`, keeping four files of up to 1 MiB by default, and the browser logs to the console without persisting anything. There is no global logger, remote backend, or automatic redaction, and guests cannot choose where logs go.

#### leptoskit_plugin_notifications: local notifications

_Packages:_ 🔴 _leptoskit_plugin_notifications_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_notifications` is the leptoskit plugin for immediate local system notifications with explicit permission requests, bounded messages, and cancellation.

Add `leptoskit_plugin_notifications` as a direct dependency and select the `notifications` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Notifications` rather than resolving the plugin on every call. iOS, macOS, and Android post through the system (Android also configures channels), while browser and Linux apps call the `web` and `linux` modules directly; Windows has no backend yet. A successful call means the OS accepted the request, not that a banner appeared, and scheduled notifications, push, and tap routing are not implemented.

#### leptoskit_plugin_local_authentication: biometrics

_Packages:_ 🔴 _leptoskit_plugin_local_authentication_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_local_authentication` is the leptoskit plugin for local device-owner authentication through Apple `LAContext` and Android `BiometricPrompt`.

Add `leptoskit_plugin_local_authentication` as a direct dependency and select the `local-authentication` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::LocalAuthentication` rather than resolving the plugin on every call. This is not account identity or server authorization; the plugin reports explicit terminal outcomes and never invents a success.

#### leptoskit_plugin_location: foreground location

_Packages:_ 🔴 _leptoskit_plugin_location_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_location` is the leptoskit plugin for foreground location permission and cancellable bounded fixes through CoreLocation, Android LocationManager, or browser Geolocation.

Add `leptoskit_plugin_location` as a direct dependency and select the `location` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Location` rather than resolving the plugin on every call. Native reads never prompt; request permission as a separate, intentional action, and the app supplies its usage description and manifest permissions. iOS, macOS, Android, and the browser are supported; Windows and Linux have no location backend yet, and there is no background tracking or geofencing.

#### leptoskit_plugin_global_shortcut: global shortcuts

_Packages:_ 🔴 _leptoskit_plugin_global_shortcut_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_global_shortcut` is the leptoskit plugin for scoped desktop global shortcuts with explicit registration conflicts and bounded events.

The native host creates `GlobalShortcuts` on its event-loop thread after startup; it has no `native.plugins` entry and installs nothing by default. `register` returns a guard that unregisters when dropped, and a host can wrap the manager in `ShortcutPlugin` so a guest with the `guest` feature registers shortcuts and polls events through `guest::GlobalShortcut`. macOS, Windows, and X11 are supported; native Wayland, browsers, and mobile return `Unavailable`, with no XWayland fallback or fake browser shortcut.

#### leptoskit_plugin_window_state: window state

_Packages:_ 🔴 _leptoskit_plugin_window_state_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_window_state` is the native-host plugin for validated window placement and explicit native window restoration.

Native hosts construct it explicitly; it is not a guest plugin and installs nothing by default. The portable state and placement API works everywhere; native capture and restore adapters currently ship for macOS, and the host owns persistence and which window is restored.

#### leptoskit_sensor: the shared sensor protocol

_Packages:_ 🔴 _leptoskit_sensor_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_sensor` is the shared protocol and native lifecycle behind the accelerometer, gyroscope, magnetometer, barometer, light sensor, and pedometer plugins.

Each request takes one fresh reading and then releases the sensor, so repeated calls are not a fixed-rate recording stream; timestamps come from the platform's monotonic clock, not UTC. The sensor plugins share its request and reply layout, cancellation rules, and browser adapters over the Generic Sensor API and device motion events, so an app can switch sensors without relearning the contract. Apps depend on the individual sensor crate; this crate is the common floor.

#### leptoskit_codepush: over-the-air updates

_Packages:_ 🔴 _leptoskit_codepush_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_codepush` ships interface changes to native apps without a store review: signed, content-hashed releases of the Wasm bundle, packed into one file, with binary deltas so a device downloads only the chunks that changed.

`leptoskit codepush` packs, signs, and publishes releases to a static channel layout that any file server, object store, or CDN can host, and `leptoskit_wasm` stages them on the device. A client accepts only releases signed by a trusted key and newer than what it runs, so a compromised registry can neither ship its own code nor roll a device back. A host can also require independent source approvals: rebuilders who built the same files from a public commit and signed to say so.

#### leptoskit_wasm: the on-device Wasm runtime

_Packages:_ 🔴 _leptoskit_wasm_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_wasm` runs the app's interface bundle on the device in `wasmi`, an interpreter that is allowed on iOS where compiling code at runtime is not, and drives a native backend through a versioned ABI.

This is what makes the shipped app a shell: the interface arrives as Wasm, renders through the same renderer, and a code push release can replace it without a store update. Every entry point is fuel-metered and rendering work is capped, so a bad bundle cannot hang the device. The generated iOS, Android, macOS, Windows, Linux, and terminal apps embed it.

#### leptoskit_axum: the server toolkit

_Packages:_ 🔴 _leptoskit_axum_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_axum` is the Axum server toolkit every leptoskit deployment shares: static files with precompressed copies and an `index.html` fallback, `/healthz` and `/readyz` probes with graceful shutdown, code push routes, and the association files that let the native apps use the site's passkeys and sign-in links.

Apps reach it as `leptoskit::server` through the `server` feature, and mount it beside `leptos_axum`; the `analytics` feature adds request measurements. It re-exports `axum`, `tokio`, `tower`, and `tower_http`, so a deployment depends on one crate, and it stays leptos-free so the toolkit composes with any router.

#### leptoskit_rpc_codegen: the RPC specification and clients

_Packages:_ 🔴 _leptoskit_rpc_codegen_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_rpc_codegen` holds the RPC specification behind leptoskit server functions and generates typed clients from it for TypeScript, Python, Go, Rust, Dart, and the command line.

The server writes the specification; the CLI reads it back and generates clients that speak the RPC's own protocol, plus an OpenAPI 3.1 document. The generators sit behind a default feature, so the server binary can depend on the types alone.

#### leptoskit_rpc_macros: RPC attributes

_Packages:_ 🔴 _leptoskit_rpc_macros_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_rpc_macros` provides the attributes behind `leptoskit_rpc`: `#[query]`, `#[mutation]`, and `#[action]` declare operations; `#[stream]`, `#[channel]`, `#[job]`, and `#[cron]` declare live streams, channels, background jobs, and scheduled jobs; and `#[api]` publishes a leptos server function to generated clients.

Use them through `leptoskit_rpc`, which re-exports them. Every target gets the operation's typed argument struct so any client can call it; only a server build keeps the function body and registers its route.

#### leptoskit_rpc: typed server operations

_Packages:_ 🔴 _leptoskit_rpc_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_rpc` is the boundary between an app's shared screens and its server. It adds retries and error classification to leptos server functions, and declares operations (`#[query]`, `#[mutation]`, `#[action]`) that run on the database as the signed-in caller and are called the same way from SSR, the browser, and a native guest through `client::call`.

The `live` feature serves queries over a WebSocket, with a server-sent events fallback, and pushes only what changed when a write commits; screens read them with `use_live`, and join channels with presence through `use_channel`. Live connections are bounded: a per-connection frame budget, a timeout for clients that stop reading, channel rooms tied to the arguments they were authorized for, and capped page sizes. `#[api]` publishes server functions so `leptoskit rpc generate` can write TypeScript, Python, Go, Rust, Dart, and command-line clients plus an OpenAPI document. Pick a database with the `postgres` or `sqlite` feature, and spread live updates across several servers with `relay`.

#### leptoskit_query: async state in the TanStack style

_Packages:_ 🔴 _leptoskit_query_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_query` is TanStack Query for leptos: screens that load data get caching, deduplication, retries, background refetching, infinite queries, and optimistic mutations, all exposed as fine-grained signals so a view re-renders only when the value it reads changes.

Declare a `QueryScope` or `MutationScope` and read it with `use_query` or `use_mutation`. Queries refetch on focus and reconnect, pause offline, stay in sync across browser tabs or native windows through `QueryClient::broadcast`, and can be fetched during server rendering with `with_ssr`. The default `browser` feature wires up the browser's timers and events; native guests enable `guest` instead, and the `db` feature refetches the queries a `leptoskit_db` live change touches. `leptoskit_rpc` builds its client calls on it.

#### leptoskit_virtual: headless virtualization

_Packages:_ 🔴 _leptoskit_virtual_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_virtual` is headless list virtualization in the style of TanStack Virtual: fixed or measured sizes, overscan, aligned scroll targets, and a window over the shared renderer.

It decides which items a viewport intersects so a ten-thousand-row list only builds the visible rows, on web and native alike. Feed a `Virtualizer` the viewport size and scroll offset, render the window with `VirtualList` (vertical or horizontal), and hand back real sizes with `measure` as items are laid out.

#### leptoskit_table: headless tables

_Packages:_ 🔴 _leptoskit_table_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_table` is headless tables in the style of TanStack Table: typed columns, sorting, filtering, pagination, and a renderable grid of ARIA roles.

It computes the model, filtering, then sorting, then paginating, while the screen owns the state: store what `for_header_press`, `for_filter`, and `for_page` propose and rebuild. Render the grid with `DataTable`, or read `rows` and draw your own with leptoskit components; either way it behaves the same on web and native.

#### leptoskit_rive: Rive playback

_Packages:_ 🔴 _leptoskit_rive_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_rive` plays Rive animations that ship inside the app: it checks a `.riv` file before loading it, picks one animation or state machine to run, and sends typed state-machine inputs (booleans, numbers, and triggers) so an animation can react to the interface.

The shared `Rive` component uses it on the web through the `web` feature and on macOS, iOS, Android, Linux, and Windows through the `native` feature; the terminal has no Rive playback. Native playback draws vector scenes only and rejects files that use images, fonts, text, layout, scripts, or audio. It never fetches anything over the network, and it should only load files you trust.

#### leptoskit_rive_runtime: the pinned Rive runtime

_Packages:_ 🔴 _leptoskit_rive_runtime_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_rive_runtime` is the pinned, offline C++ dependency used by `leptoskit_rive` native playback.

Applications use `leptoskit_rive`; this crate exposes no application API. Its separate Cargo build lets changes to the Rust player or image adapter reuse the C++ compilation cache.

#### leptoskit_docs: documentation sites

_Packages:_ 🔴 _leptoskit_docs_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_docs` turns Markdown with leptos components into a static site with navigation, search, highlighting, and checked links.

The leptoskit book is built with it, and apps can use the same generator for their own docs. It is MDX-shaped but Rust-native: components are leptos components, and mistakes fail the build with a location.

#### leptoskit_macros: view!

and #[component]

_Packages:_ 🔴 _leptoskit_macros_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_macros` provides the `view!` markup macro and `#[component]` attribute that expand into the same builders a hand-written tree uses.

Use them through `leptoskit::prelude`. The macro parses rstml markup and emits builder calls, so markup and builder code produce one element tree and neither is a second-class path.

#### leptoskit_router_macros: the Route derive

_Packages:_ 🔴 _leptoskit_router_macros_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_router_macros` provides the `#[derive(Route)]` macro that reads a route struct's fields from the URL by name.

It is an implementation detail of `leptoskit_router`, which re-exports the derive. `Option<T>` may be absent, `Vec<T>` takes every value, and other types are required unless the field is `#[route(default)]`.

#### leptoskit_router: typed file routes

_Packages:_ 🔴 _leptoskit_router_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_router` is the runtime behind leptoskit file routes: each file in `src/routes/` becomes a variant of one generated `Route` enum, so URLs are parsed into typed values and links are written from them, and the two cannot drift apart.

`leptoskit dev`, `leptoskit build`, and `leptoskit routes` generate the enum; the app then holds a `Router` to read the current route, `navigate`, `replace`, go `back`, and build links that render as real `<a href>` links on the web. Each page renders inside the layouts of the folders above it. Natively the history lives in memory; the `web` feature keeps the browser's address bar and back button in step. Routes match from most to least specific, and parameter structs use `#[derive(Route)]`.

#### leptoskit_plugin_router: file routes as a build plugin

_Packages:_ 🔴 _leptoskit_plugin_router_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_router` is the file router as a leptoskit plugin: it generates the typed `Route` module from the files under `src/routes/` before cargo runs, and answers `plugin:router routes`.

Turn it on with a `[[plugins]]` entry named `router` in `leptoskit.toml`; `leptoskit dev`, `build`, and `native` then rewrite `src/routes.rs` only when the routes change, and `leptoskit plugin:router routes --check` fails when the committed module is stale. Apps don't depend on this crate, since the CLI links it; the runtime and the `Route` derive live in `leptoskit_router`.

#### leptoskit_router_codegen: the routes generator

_Packages:_ 🔴 _leptoskit_router_codegen_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_router_codegen` reads a leptoskit `src/routes/` directory and writes its typed `routes.rs`.

The `leptoskit routes` command and the CLI's build pipeline use it; apps never depend on it. It reports every problem with its file and line, and never overwrites a file leptoskit did not write.

#### leptoskit_db_schema: schema diff and migrations

_Packages:_ 🔴 _leptoskit_db_schema_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_db_schema` holds the schema model, snapshot format, diff, and SQL renderers behind leptoskit's generated database migrations.

`leptoskit db` and the `leptoskit_db` runtime use it; apps never depend on it. It is pure - no connection and no files - and renders Postgres and SQLite SQL from the same plan, with each step's risk assessed against existing rows.

#### leptoskit_db_macros: the database macros

_Packages:_ 🔴 _leptoskit_db_macros_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_db_macros` provides the macros behind leptoskit's database layer: `#[db::table]`, `#[db::link]`, `#[db::view]`, `#[db::policy]`, `#[db::mixin]`, `#[db::polymorphic]`, `db::migrations!`, and the `Shape`, `Enum`, and `Scalar` derives.

Apps use them through `leptoskit::db` or `leptoskit_db`; this crate is the macro implementation. Every mistake it can catch is a compile error that says what to write instead.

#### leptoskit_db: tables as Rust structs

_Packages:_ 🔴 _leptoskit_db_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_db` is leptoskit's database layer: tables declared as Rust structs, typed IDs, client-safe public projections that leave out private and secret columns, typed queries and writes, links, access policies, and migrations.

Enable `db` on `leptoskit` for the client side, which has no database code, and `db-postgres` or `db-sqlite` on the server; `db-geo` and `db-vector` add geospatial and vector columns. `leptoskit db` diffs the tables and writes migrations, and `leptoskit db studio` browses the data in development. Auth and IDP build their schema packs on it.

#### leptoskit_auth: sign-in and sessions

_Packages:_ 🔴 _leptoskit_auth_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_auth` is sign-in for leptoskit apps, stored in the app's own database: the `auth` schema pack, revocable sessions, email codes, authenticator apps with recovery codes and step-up, passkeys, third-party providers, opt-in passwords, Solana wallets, account merging, and the signed-in user that access policies read.

Enable `auth` on `leptoskit`, plus `auth-passkeys`, `auth-providers`, `auth-passwords`, `auth-solana`, `auth-smtp`, or `auth-webhook` in the server's crate for the parts that need them. The headless client works on the web and in the generated iOS, Android, and macOS apps, where it uses the passkeys, web-auth, secure-store, and HTTP plugins. `leptoskit_idp` turns the same accounts into an OIDC provider.

#### leptoskit_idp: an OIDC provider for your app

_Packages:_ 🔴 _leptoskit_idp_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_idp` lets other apps sign in with your app: an OAuth 2.1 and OpenID Connect provider built on the app's own database, sessions, and access policies.

It turns the accounts `leptoskit_auth` already manages into an identity provider, without a second user store. Authorization, consent, and token issuance stay in the app.

#### leptoskit_extension: browser extensions

_Packages:_ 🔴 _leptoskit_extension_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_extension` is the runtime and typed WebExtensions bindings behind leptoskit browser extensions for Chromium, Firefox, and Safari.

Apps enable the `extension` feature, name entry points with `entry!`, and build with `--platform extension`. APIs return `Unavailable` instead of throwing, and listeners are removed when their handle drops.

#### leptoskit_devtools_protocol: the DevTools wire format

_Packages:_ 🔴 _leptoskit_devtools_protocol_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_devtools_protocol` holds the versioned serde messages shared by the DevTools agents, hub, interface, and MCP tools.

It has no platform code: the browser, native host, guest, and front end all speak the same protocol. A version mismatch is refused and both versions are named.

#### leptoskit_plugin_devtools: the guest-to-host DevTools bridge

_Packages:_ 🔴 _leptoskit_plugin_devtools_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_devtools` is the bridge plugin that carries a native guest's DevTools data to the host that already reports the tree.

Apps never select it. The shared runtime installs it in `leptoskit dev` builds, and the host merges the guest's route, state, forms, and queries into the session it reports.

#### leptoskit_devtools: the DevTools agents

_Packages:_ 🔴 _leptoskit_devtools_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_devtools` contains the agents that report a running app to the local DevTools hub: the web page, the native host, and the Wasm guest.

Under `leptoskit dev`, the CLI starts the hub and these agents open sessions, mirror trees, and carry commands back. Hooks expose the route, named state, forms, and query caches that only the app knows.

#### leptoskit_l10n: typed translations

_Packages:_ 🔴 _leptoskit_l10n_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_l10n` provides typed translations in the style of Flutter's `gen_l10n`: locales, negotiation, and plural rules behind a generated messages module.

Messages live in one ARB file per locale; `leptoskit l10n` writes the typed module, and app code calls getters instead of parsing ARB at runtime. It works in SSR, the browser, and native guests.

#### leptoskit_l10n_codegen: the translations generator

_Packages:_ 🔴 _leptoskit_l10n_codegen_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_l10n_codegen` reads a leptoskit `locales/` directory of ARB files and writes its typed translations module.

The `leptoskit l10n` command and the CLI's build pipeline use it; apps never depend on it. It checks every translation against the template and reports placeholder, plural, and naming problems with a file and line.

#### leptoskit_cli: the command line

_Packages:_ 🔴 _leptoskit_cli_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_cli` is the `leptoskit` and `lk` command: scaffold a project with `new`, run every platform at once with `dev`, then `build`, `serve`, and `test`. It also writes routes, translations, and database migrations, generates sign-in screens and plugins, ships code push releases, uploads to the app and extension stores with `publish`, drives devices and simulators (also over MCP), and lints and formats the code.

Deployment lives here too: `infra` manages OpenTofu infrastructure, `deploy` releases a commit or rolls back, `console` hosts or connects to the self-hosted deployment service, or pairs a worker with the hosted LeptosKit app, and `verify` reproduces backend builds and checks signed deployment policies. New projects resolve their development secrets through Monosecret, and `leptoskit dev --no-secrets` uses the existing environment. The binary is a thin wrapper over the library, so tooling can drive the same commands directly.

#### leptoskit_fmt: the view!

formatter

_Packages:_ 🔴 _leptoskit_fmt_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_fmt` is the formatter behind `leptoskit format`: rustfmt for Rust and a layout engine for the markup inside `view!`.

rustfmt leaves `view!` markup alone, so this fills the gap without a second Rust formatter: it lays markup out with the project's `rustfmt.toml` width and indentation, copies text exactly, and keeps every comment. The `[format]` table in `leptoskit.toml` sets its style, `--check` fails CI when a file would change, and `--stdin` serves editors.

#### leptoskit_lint: project conventions

_Packages:_ 🔴 _leptoskit_lint_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_lint` contains the rules behind `leptoskit lint`: components carry `#[component]`, and component trees follow the markup style `leptoskit.toml` selects.

rustc and clippy check Rust; these rules check the conventions a leptoskit project relies on, with diagnostics that name the file, line, and fix.

#### leptoskit: the one dependency

_Packages:_ 🔴 _leptoskit_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit` is the crate an app depends on. It re-exports the components, Tailwind styles, layout engine, renderer, and Wasm runtime, with one feature per optional part, so an app names `leptoskit` alone and never keeps the versions of its parts in step by hand.

Pick how the interface runs: `web` with `csr`, `hydrate`, `ssr`, or `islands` renders through leptos, and a backend feature (`macos`, `ios`, `android`, `linux`, `windows`, or `tui`) commits the same tree as real widgets or as terminal cells. `server` adds the Axum toolkit, and `analytics` beside it keeps request and browser measurements. Everything else is opt-in: `rpc`, `query`, `form`, `aria`, `table`, `virtual`, `l10n`, `rive`, `db` with `db-postgres` or `db-sqlite`, `auth` (passwords, passkeys, and providers through their own `auth-*` features), `idp`, `extension`, `codepush`, and `devtools`. `leptoskit::secrets` reads values that Monosecret, the default secret manager, or a trusted host supplies.

### Features

#### Deploy, back up, and roll back on your own servers

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #505](https://github.com/leptoskit/leptoskit/pull/505)

Describe your servers in an `[infra]` table in `leptoskit.toml`, and `lk infra` (also `lk tofu`) and `lk deploy` take the app from there to running Linux hosts. They provision Hetzner or DigitalOcean servers behind a Cloudflare Tunnel, an R2 bucket that serves signed code push updates, and a private R2 or Backblaze bucket for backups, then run the app and Postgres in containers with Kamal:

```sh
lk infra generate
lk infra plan
lk infra apply
lk infra build --version release-001 --push
lk deploy --setup --version release-001
```

- `lk infra apply` applies only the plan you saved and reviewed, and refuses a configuration that changed since. `lk infra offers` lists current prices, and `[infra.budget]` stops a plan whose compute would cost more per month than you allow.
- Several app hosts update one at a time. An optional NATS relay for live connections and a bounded Valkey cache run on the private network; Postgres, NATS, and Valkey stay on one primary host.
- Scheduled backups are encrypted with Restic, restored into a throwaway database to check them, and pruned by your retention settings. `lk infra restore` keeps the previous database beside the restored one.
- Each image carries its migration history, which is checked before the app starts, and each release records its image digest. `lk deploy --rollback <tag>` checks both against the live database. Rolling back a schema change needs reviewed `down.sql` files, stopped writers, and `--rollback-schema --yes`; a failed schema change is never reversed automatically.
- `lk infra ci` writes a GitHub Actions workflow that builds an image for each push to `main`, tagged with its commit, and deploys it.

`lk db migration create` now writes a `down.sql` beside each new `migration.sql`; existing migrations and their checksums are left alone. `lk db rollback` and `Db::rollback` reverse applied migrations newest first, each in its own transaction, and refuse to start when a reverse is missing or still marked `-- leptoskit: irreversible`.

`leptoskit generate deploy` also deploys a client-rendered app whose backend and browser client are separate packages (`build.client_package`). A NATS relay URL that carries a user name and password now passes them to the client instead of to the connection address.

#### Measure your deployed app with the analytics package

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #505](https://github.com/leptoskit/leptoskit/pull/505)

When a deployed route starts failing, you need its error counts, latency, and release in one place. Turn on the `analytics` feature of `leptoskit` on your server and add `[infra.plugins.analytics]` to `leptoskit.toml`:

```toml
[infra.plugins.analytics]
version = 1
environment = "production"
browser_routes = ["/", "/notes/{id}"]
```

The server then measures every HTTP request in bounded per-minute windows inside its own process, with W3C request correlation. `server::Timer` measures a named database, RPC, job, or live operation. An optional browser module records page views, navigation timing, and error counts for the routes you list, without cookies, query strings, or user identifiers, and keeps those reports apart from the server's own measurements. `lk infra analytics` reads one instance's measurements through a key-protected JSON API, and `lk infra plugins` lists what the plugin needs. It adds no containers or cloud resources.

To keep history across instances, connect the app to a deployment service, or set `otlp_endpoint` to export them to an OTLP/HTTP metrics backend.

#### Run your own deployment service

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #505](https://github.com/leptoskit/leptoskit/pull/505)

`lk console serve` runs one service, on a trusted Linux or macOS worker, that holds the deployment dashboard, a queue of deployment jobs, an API for agents, and the analytics history of every instance of your app. Connect an app once, and the ordinary commands run through it, so a deployment keeps going after you close your terminal:

```sh
lk console serve --project /srv/notes --url https://deploy.example.com   # on the worker
lk console connect --url https://deploy.example.com                       # in your checkout
lk infra plan
lk infra apply
lk deploy
```

It reuses the `lk infra` and `lk deploy` engine. Credentials stay on the worker and in a private file outside your repository. `apply` runs only the plan this checkout reviewed, and `lk deploy` builds the current Git commit in a temporary checkout on the worker. An operation interrupted by a restart is marked as such and waits for you to reconcile it, rather than being retried blind. `lk console token` issues project-scoped `read`, `deploy`, and `ingest` keys for agents and apps.

Apps with the analytics plugin get their ingestion key and endpoint automatically. The service keeps up to seven days of measurements per project, alongside process, cgroup, and filesystem samples from each host, and can export metrics to an OTLP/HTTP backend instead. `lk console backup` and `lk console restore` back up and restore the service's own database; a restore revokes the keys it brings back.

#### Manage deployments from a hosted account

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #505](https://github.com/leptoskit/leptoskit/pull/505)

A hosted LeptosKit account keeps deployment requests, application measurements, and OpenTofu state in one place, managed from one app that runs on the web, iOS, and Android. Accounts are created by the service's operator, and subscriptions go through Stripe Checkout and the Stripe customer portal.

Deployments run on a dedicated worker that connects out to the service, so it needs no public address. Pair it with a one-use code from the app, then use the usual commands:

```sh
lk console worker --url https://deploy.example.com --project /srv/notes --data /srv/notes-worker
lk console connect --url https://deploy.example.com
lk deploy
```

Each account's data and keys are kept apart, and no two customers share a worker. With `infra.hosted_state = true`, OpenTofu state is stored encrypted, versioned, and locked by the service. Agent keys can deploy but can't manage billing, pair workers, or apply infrastructure changes. Workers run on your own infrastructure, or the operator can provision a dedicated DigitalOcean installation for you after reviewing its configuration and cost. A worker that stops partway through an operation doesn't repeat it automatically; it waits for the operator to reconcile it.

#### Native packages and a per-platform doctor

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #521](https://github.com/leptoskit/leptoskit/pull/521)

Plugins can now wrap third-party native SDKs the way the platform expects, and apps never run a package manager by hand.

A plugin's Swift Package Manager dependencies were already declared in its `Package.swift`; they now resolve automatically on both iOS and macOS, documented as the way to wrap SDKs like Firebase's Swift package. A library that ships only as a pod is declared in the plugin's runtime metadata instead:

```toml
apple-pods = [
	{ name = "FirebaseCore", version = "~> 12.0" },
]
```

leptoskit generates the Podfile, runs `pod install` before every iOS build (and before an App Store archive), and compiles the plugin's Swift into the app target where the pods are linked. CocoaPods is required only while a selected plugin declares pods — never otherwise — and `leptoskit doctor` reports exactly when it is. Pods are iOS-only; macOS builds with Swift packages.

`leptoskit doctor` now checks each native platform the project targets: Xcode, the iOS simulator runtime, and CocoaPods on iOS; Swift on macOS; Java 17+, the Android SDK, platform 36, an NDK, CMake, and adb on Android; GTK 4 through pkg-config on Linux; plus every Rust target each platform compiles for. Inside a project it reports only the platforms the app ships; `leptoskit doctor --all` checks everything this machine could build. `leptoskit new` now points at it.

Native builds are also quieter where the tools shout and clearer where they were silent: xcodebuild runs with `-quiet` and without index-store chatter, Gradle runs with a plain quiet console, and the Linux, Windows, terminal, and web builds no longer drop the Rust compiler's warnings — and errors — that cargo's JSON message stream carried.

`android-dependencies`, the equivalent Maven mechanism for Android plugins, is now documented alongside.

- **Record and check offline backend bundles.** Add `leptoskit verify record` and `leptoskit verify check` to inventory prepared backend source, dependencies, infrastructure, build declarations, secret names and executable files. Checks require an independently trusted inventory SHA-256 and reject changed, missing or extra entries and symbolic links. These commands check local bundle integrity; they do not attest the build or verify a live backend. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #508](https://github.com/leptoskit/leptoskit/pull/508)

#### Require independent source approvals for code push releases

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #511](https://github.com/leptoskit/leptoskit/pull/511)

Add `[codepush.provenance]` so native apps install a code push release only when enough independent approvers say they rebuilt it from one commit of a public GitHub repository and got byte-identical files. Approvals are a second trust root, separate from the publisher keys: each is signed by an approver over its exact statement, under its own signing context, so a publisher signature can't stand in for one. Devices check approvals before downloading a release and again before every launch, refuse approvers that name different commits, count each approver once, and ignore approvals from any other key, including the publisher's. The bundle shipped in the store build needs none, and without the table nothing changes.

Add `leptoskit codepush approve`, which signs an approval only when a rebuilt directory holds exactly the release's files, `leptoskit codepush attach`, which adds approvals to stored releases without re-signing them, and `leptoskit codepush keygen --approver`. `publish` and `serve` carry each release's approvals at `approvals/<bundle hash>.json`. An approval says who rebuilt identical files from a commit; it is not a security review.

#### Amend drafting between releases

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #530](https://github.com/leptoskit/leptoskit/pull/530)

`[db] draft = "amend"` makes `leptoskit db migration create` rewrite the newest unreleased migration from the snapshot before it instead of stacking a new one, so between releases there is one draft that keeps up with the tables — and the release's `migration consolidate` has one entry to fold or nothing to do. Data migrations (`--empty`) still stack, and a development database that applied the draft's earlier shape is told to run `db reset --yes` to come back in line.

#### Migration consolidation at releases

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #528](https://github.com/leptoskit/leptoskit/pull/528)

`leptoskit db migration consolidate` folds every migration after the latest release marker into one, written from a fresh diff of the release's baseline to the current tables: add-then-drop vanishes, rename chains collapse, and intermediate SQLite table rebuilds never happen. The consolidated migration carries a `replaces.json` naming what it folded, and every database converges on its own — one that already applied the folded migrations records the consolidated one without running it, while a fresh one runs it once; a database that applied only part of the set is refused by name. A migration whose SQL no longer matches what its snapshots generate — a data migration, or one written by an older leptoskit — is refused unless it is named in `--keep`, which leaves it and the migrations after it in place, after the consolidated one.

- **Filters that follow links, totals, and distinct reads.** `Query::aggregate` totals a column over the matching rows — `Sum`, `Avg`, `Min`, or `Max` of a column in one typed statement, with the table's policies applied and `None` when no row matches, as SQL returns. A link column's `has` follows it: `post.author_id.has(|user| user.handle.eq("ada"))` becomes a correlated `EXISTS`, in `filter` and in `!` for its negation, and an optional link that is `NULL` matches nothing. Text columns match regular expressions with `matches_regex` — Postgres's `~`; queries on SQLite refuse it rather than diverge. `distinct()` reads each row once, which matters for views and naturally-keyed tables, and `count()` counts after it. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #533](https://github.com/leptoskit/leptoskit/pull/533)

#### Release markers for database migrations

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #527](https://github.com/leptoskit/leptoskit/pull/527)

`leptoskit db release record <version>` writes a marker into `migrations/registry.txt` that freezes every migration above it as published, pinning the SHA-256 of the schema the release ends at. Editing a frozen migration's snapshot is refused the next time the migrations are read, and a marker with a typo in it never silently unfreezes a release. `leptoskit db status` names the release that froze what, and how many migrations came after it; those are the drafts the next release will consolidate.

- **Wall-clock times, durations, and sequences.** Columns can now hold a `chrono::NaiveDateTime` or `chrono::NaiveTime` — a calendar's date and time or a time of day in no time zone, `timestamp` and `time` on Postgres and ISO 8601 text on SQLite — and a `chrono::Duration`, stored as ISO 8601 duration text (`PT90M`, `-PT36H`) on both databases, so a grace period or a lead time round-trips exactly, sign and fraction included. `#[db(sequence)]` on an `i64` makes the database number the rows in order (`GENERATED BY DEFAULT AS IDENTITY` on Postgres); inserts don't name the column, and SQLite projects are told to key by an `Id` or keep a counter of their own, with a way out in the message. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #535](https://github.com/leptoskit/leptoskit/pull/535)
- **Sign which origins serve a deployment.** Add `leptoskit verify keygen`, `leptoskit verify sign`, and `leptoskit verify deployment`. A project signs a deployment policy listing its origins and the verified releases they may serve, and serves it at `/.well-known/leptoskit/deployment.json`. Anyone can check an origin against the project's published key: a copied website fails because its origin is not listed, a forged policy fails its signature, and `--state` refuses an older or conflicting policy. The check does not show what a server actually runs. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #510](https://github.com/leptoskit/leptoskit/pull/510)
- **Develop and test extensions in Firefox.** Select Firefox with `--browser firefox` when running `leptoskit dev` or `leptoskit test extension`. Development builds use a temporary add-on installed through WebDriver BiDi, reload it after a rebuild, and check page surfaces, background startup, and content script injection. `leptoskit doctor` reports Firefox and geckodriver. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #488](https://github.com/leptoskit/leptoskit/pull/488)

#### Add password fields to the generated sign-in screens

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #494](https://github.com/leptoskit/leptoskit/pull/494)

With `[auth.passwords] enabled = true`, `leptoskit generate auth` now writes password fields into the screens. The sign-in screen offers "Sign in with a password" and resets a forgotten password with an emailed code; the security screen sets, changes, and removes the password, with the server's minimum length beside the field. The fields are `PasswordInput`s marked `current-password` or `new-password`, so password managers fill and save them on the web and in native apps.

Turning passwords on later and running `leptoskit generate auth` again adds the fields to screens you haven't changed, and leaves the ones you have alone unless you pass `--force`. Turning them off never takes fields away.

In app code, `leptoskit::auth::client::sign_in_with_password()` takes an `EmailPassword` (the address and password), alongside `EmailCode` for emailed codes.

#### Package the hosted pilot for a Linux server

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #507](https://github.com/leptoskit/leptoskit/pull/507)

Build the broker, public site, console and documentation as one checksummed Linux release, with a systemd service that keeps persistent data separate from releases. Drain broker requests on termination so service restarts can finish active requests.

Use `[infra.external]` to provision a separately supervised service with the existing OpenTofu integration, including DigitalOcean, Cloudflare routing, the price guard and saved-plan protection, without requiring the managed Postgres/Kamal runtime.

#### Serve the docs book under a strict CSP

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #505](https://github.com/leptoskit/leptoskit/pull/505)

Two `leptoskit_docs::Site` changes let you serve the book from your own site:

- `Site::external_fonts(false)` stops the book loading its fonts from Google Fonts, for a site that supplies local fonts in its own stylesheet or has to work offline.
- The script that applies a reader's saved light or dark theme is now a same-origin file instead of an inline script, so the book works under a content security policy that forbids inline scripts.

The LeptosKit website is built this way: it serves the landing page, a live shared component example, the deployment console at `/console/`, and the whole book at `/docs/`, with local fonts and the Rive runtime packaged alongside. The management app's **Documentation** button opens that book on the web, iOS, and Android.

#### Use Monosecret for development secrets

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #487](https://github.com/leptoskit/leptoskit/pull/487)

New projects declare their database and sign-in secret names in `monosecret.toml`. `leptoskit dev` resolves that manifest through Monosecret before starting, with `--no-secrets` for hosts that supply the environment. Existing projects without a manifest keep their current startup. The secret management guide recommends Monosecret for server and deployment credentials.

`leptoskit::secrets` exposes Monosecret 0.5's small value-access client on native and Wasm targets. Native servers can read named environment values; Wasm hosts provide values explicitly. Default provider and runtime features are disabled, keeping the resolver, network clients, and provider dependencies out of application bundles.

#### Draw password fields in generated apps

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #494](https://github.com/leptoskit/leptoskit/pull/494)

The macOS, iOS, and Android apps `leptoskit new` generates now draw `PasswordInput`, as an `NSSecureTextField`, a secure `UITextField`, and a password `EditText`. Before, a password field stopped those apps with an "unsupported widget" error.

`autocomplete` hints now reach native password managers too. `username`, `email`, `current-password`, and `new-password` set the field's content type on macOS and iOS, its autofill hint on Android, and its input type on HarmonyOS, in the generated apps and in the AppKit, UIKit, and Android backends. `NativeProps::text_content` reads the hint as a `TextContent`. A `PasswordInput` turns off autocorrection, spell checking, and suggestions on iOS and Android, and an Android password field without a hint is offered to autofill as a password.

- **Build and reproduce backends from public commits.** Add `leptoskit verify build`, which builds a backend from a public GitHub commit in a digest-pinned container: the commit's exact files, its locked dependencies, and two compilations with networking disabled that must produce identical executables. It writes the bundle, its inventory, and a SLSA provenance statement. `leptoskit verify reproduce` lets anyone fetch the same commit and dependencies, rebuild, and confirm the published executable byte for byte. Neither command shows what a live server runs. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #509](https://github.com/leptoskit/leptoskit/pull/509)

#### Add generated artwork to the documentation book

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #505](https://github.com/leptoskit/leptoskit/pull/505)

`leptoskit_docs::Site::generated_asset` adds a file your build generates, such as SVG artwork or a stylesheet, to the book under `_leptoskit/generated/`, without writing it into the book's sources. The file is kept through development-server rebuilds, and an invalid file name fails the build before any output changes.

The LeptosKit website and book use it for their hand-drawn look: borders, hatching, and artwork drawn at build time with skribble's rough engine, in a paper-and-ink palette for light and dark modes, with locally served fonts.

#### A GPU canvas for custom drawing

_Packages:_ 🟠 _leptoskit_canvas_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #545](https://github.com/leptoskit/leptoskit/pull/545)

The new `leptoskit_canvas` crate draws the kind of graphics no native widget offers, such as charts, progress rings, and cards with soft shadows and gradients. You describe the drawing as a `Scene`, an ordered list of commands in the style of React Native Skia, and its renderer draws it on the GPU with wgpu (Metal on Apple platforms, Vulkan on Linux and Android, Direct3D 12 on Windows, and WebGPU in the browser).

A scene can fill rectangles with a radius per corner and an optional border, cast CSS-style box shadows, paint with solid colours or linear and radial gradients, fill vector paths with the non-zero or even-odd rule, stroke them with caps and joins, and nest clips, transforms, and layers drawn at an opacity. Edges stay smooth at any scale. A scene is plain data, so a Wasm guest can build one and send it to its host, and the renderer bounds every value before drawing, so a scene from untrusted code cannot make the host allocate without limit.

The crate is standalone in this release: no component uses it yet. `render_to_rgba` draws a scene offscreen and returns its pixels, which suits tests and generated images.

- **Theme tokens in leptoskit.toml and deeper CSS var().** `[theme.colors]` and `[theme.lengths]` in `leptoskit.toml` name design tokens that `Theme::install_maps` loads into the process-wide Theme map. Arbitrary color and length utilities accept `var(--name)`, one nested `var()` fallback, and `/opacity` on color vars. The web adapter prepends `Theme::css()` so the browser sees the same `:root` custom properties native resolves through Theme. Tailwind `@theme` / `tailwind.config` files remain unsupported. _Packages:_ 🟠 _leptoskit_native_, 🟠 _leptoskit_config_, 🟠 _leptoskit_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #538](https://github.com/leptoskit/leptoskit/pull/538)
- **CSS color variables and a Theme map.** Arbitrary color utilities can use `var(--name)` or `var(--name, fallback)`. The web stylesheet keeps the CSS function. Native paint resolves the name through `Theme::set_color` (or the fallback when the theme has no entry). `Theme::css` emits a `:root` sheet of the same names for the browser. Nested `var()`, non-color variables, and opacity modifiers on `var()` stay unsupported. _Packages:_ 🟠 _leptoskit_native_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #536](https://github.com/leptoskit/leptoskit/pull/536)

#### Try native plugins written only in Rust

_Packages:_ 🟠 _leptoskit_config_, 🟠 _leptoskit_bridge_, 🟠 _leptoskit_bridge_ffi_, 🟠 _leptoskit_plugin_clipboard_, 🟠 _leptoskit_plugin_haptics_, 🟠 _leptoskit_cli_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #506](https://github.com/leptoskit/leptoskit/pull/506)

Set `native.experimental-rust-plugins = true` in `leptoskit.toml` to use the experimental pure-Rust `clipboard` and `haptics` plugins on iOS, macOS, and Android in place of their Swift and Kotlin versions. Without the setting, builds keep the Swift and Kotlin versions.

A plugin of your own can do the same in its `[package.metadata.leptoskit.runtime]` tables. With `rust-context = true`, its factory receives a `leptoskit_bridge::AppContext`, whose `platform()` lends the host's native handles: the `JavaVM` and app `Context` on Android, and the on-screen view controller on iOS and macOS. With `rust-main-thread = true`, its calls run on the platform's UI thread, which UIKit, AppKit, and most Android UI APIs require (`App::install_main` installs such a plugin by hand). Mark a platform's registration `experimental = true` to use it only when the setting is on.

### Fixes

- **Preview the deployment dashboard with example data.** `leptoskit_deploy_console` keeps the dashboard's original design study beside the working deployment service. Its `model`, `store`, and `view` modules and its `preview` example render the dashboard's screens (deployment history, rollback and schema checks, deployment targets) from built-in example data, and export them as an interactive HTML page. The preview needs no server or credentials and never touches your cloud accounts; to deploy, run the service with `lk console serve`. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #505](https://github.com/leptoskit/leptoskit/pull/505)
- **Render Android form controls and dismiss the keyboard.** Generated Android apps now render Switch, Checkbox, Radio, Slider, and progress indicators using native Android widgets. The shared native event bridge accepts checked-state and numeric-value events. Tapping outside an editor clears focus and hides the keyboard in both Android renderers. Generated apps move between inputs with the keyboard Next action and dismiss the keyboard on Done. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #519](https://github.com/leptoskit/leptoskit/pull/519)

#### Render common controls in generated Apple apps

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #520](https://github.com/leptoskit/leptoskit/pull/520)

Generated iOS and macOS apps now render switches, checkboxes, radio options, sliders, progress bars, and activity indicators. Preserve their state through binary transactions, send toggle and value-change proposals to the guest, and expose native selection and value accessibility.

Tapping outside an input dismisses the keyboard while preserving button actions. Return ends editing and submits; guest-controlled focus can move to another field. Scroll views support interactive keyboard dismissal.

Focus requests on iOS no longer reopen the keyboard after dismissal on an unrelated render. macOS controls restore the state accepted by the guest after each proposal.

- **Field keeps its draft, and the gallery fits a phone.** Typing in Field's compose screen no longer clears each keystroke: the screen now owns its draft, keeps it when you leave the editor and come back, and has a Back button. The gallery's browser preview now fits the available width, so its controls stay reachable on phone-sized screens. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #518](https://github.com/leptoskit/leptoskit/pull/518)

#### Harden sign-in, merges, passwords, and the provider

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #498](https://github.com/leptoskit/leptoskit/pull/498)

`leptoskit_auth`:

- Starting a passkey or wallet sign-in no longer cancels everyone else's. These sign-ins share one subject before anyone is known, so a script that kept starting them could break sign-in for every visitor.
- An account that is disabled or locked can no longer be merged into another. Before, merging let a banned account shed its ban, and let someone take guesses at a locked account's second factor.
- Setting or changing a password counts against the account's password limit. A password reset counts against the per-IP limit before it asks the breach service. Argon2 runs at most two hashes per core at once.
- A refresh token traded again within the 30-second grace works only once. If a thief makes that trade, the app's own token ends the session when it is next used, and `refresh_reused` is recorded.

`leptoskit_idp`:

- Scope lists are de-duplicated in linear time; before, a long list cost quadratic time.
- Every authorization parameter is capped at 4 KiB, so pushed requests can't store megabytes.
- A DPoP proof or client assertion with an extreme `iat` or `exp` is refused instead of overflowing.
- A code redeemed by two requests at once ends the tokens the winning request got, as a later replay already did.

#### Harden the CLI against web pages and leaked secrets

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #489](https://github.com/leptoskit/leptoskit/pull/489)

- `leptoskit mcp http` now refuses requests a web page could send: any request with an `Origin` or `Sec-Fetch-Site` header, a `POST` whose content type isn't `application/json`, and a `Host` other than the address it listens on. Before, a site open in your browser could post to it and run the `control` tool. Send `content-type: application/json` with every message. Binding it to a non-loopback `--address` now prints a warning, since it has no authentication.
- The Firefox source archive leaves out symlinks that point outside the project, and `.jks`, `.keystore`, and `.pfx` files, so `leptoskit publish firefox` can't upload a linked credential or an Android keystore.
- A malformed publish credentials file reports the line that is wrong, without quoting it, so the secret on that line isn't printed.
- `leptoskit publish macos` names its `.app`, `.zip`, and `.dmg` from the app's name with path separators replaced, so a name like `AC/DC` stays in the output folder instead of naming a path elsewhere.
- Branch database names no longer panic when a long app name has a multi-byte character where the name is cut.
- The release workflow `leptoskit new` writes now reads the default branch from the event (`$GITHUB_DEFAULT_BRANCH` was never set, so the check failed every tagged release), and checks that manual runs start from a commit on the default branch too.

#### Fix a query deadlock, stale async results, and routing

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #493](https://github.com/leptoskit/leptoskit/pull/493)

Fixes in the client libraries:

- `leptoskit_query`: a watcher that reacts to a query starting to fetch, by cancelling, refetching, or invalidating it, no longer freezes the app. The fetch used to hold a lock while it notified watchers.
- `leptoskit_use`: `use_async_state` keeps the newest result. When two runs overlap and the older one finishes last, its result no longer replaces the newer one.
- `leptoskit_router`: a route under a catch-all, such as `[...path]/edit.rs`, is now reachable. `[...path]/index.rs` used to match `/a/edit` first.
- `leptoskit_l10n`: `negotiate` keeps looking through every following preference in the same language before settling for a language-only match. `[en-GB, en-AU, en-US]` now gets `en-US` from `[en, en-US]` instead of `en`.
- `leptoskit_virtual`: measuring an item at its estimated size no longer recomputes the position of every item after it. Measuring the rows that scroll into view in the middle of a 100,000-item list used to recompute 50,000 positions; now it recomputes none.
- `leptoskit_aria`: `Field::wire` replaces the field's own help and error references instead of adding them again, so wiring an input twice no longer makes a screen reader read its help twice, and an error the field no longer shows is no longer referenced.

- **Refuse older code push releases after losing the newest.** In the generated iOS and Android apps, a device remembers the newest update it has confirmed in a file of its own. Before this change, a damaged update file or a disk error made the device forget it had ever updated, so a stale or hostile `index.json` could install an older release that was still validly signed. The device now refuses anything at or below the newest release it confirmed, until a new store build of the app ships its own bundle. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #500](https://github.com/leptoskit/leptoskit/pull/500)

#### Close an upsert race and keep large writes working

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #501](https://github.com/leptoskit/leptoskit/pull/501)

- An upsert under row policies now locks the row it would overwrite before checking the update policy, and inserts plainly when there is none. Before, on Postgres, an upsert racing another transaction's insert of the same unique value could overwrite that row, `author_id` included, when only the insert policy had been checked.
- A commit of more than 500 rows to a SQLite table with a read policy no longer breaks every live feed that checks that policy. SQLite allows at most 500 terms in one compound `SELECT`.
- Bulk updates, soft-delete cascades, and supertype removals check and change their rows in batches of 500. They no longer fail past the databases' bind limits, or overflow the stack on tables keyed by two columns.
- The studio no longer filters or sorts by `secret` columns, which could read a secret back one character at a time.

#### Move the repository to the `leptoskit` organization

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #522](https://github.com/leptoskit/leptoskit/pull/522)

The project now lives at `leptoskit/leptoskit`. Crate metadata, repository links, badges, the documentation's repository link, and the release workflow's trusted-publishing identity name the new organization.

The public site stays at <https://leptoskit.dev/>, and configuration schema URLs are unchanged. Release downloads and the installed agent skill keep working through GitHub's repository redirects.

- **Allow hosted accounts to subscribe again after cancellation.** Bind replacement Checkout completion to the pending reservation before selecting the new subscription. Delayed events from the old subscription no longer reject the new one or clear its pending checkout, and repeated deliveries remain idempotent. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #507](https://github.com/leptoskit/leptoskit/pull/507)
- **Return from signing out only to the app that asked.** `leptoskit_idp`: `/oauth/end_session` follows `post_logout_redirect_uri` only when the app named by the ID token in `id_token_hint` registered it, or, without an ID token, the app named by `client_id`. In every other case people land on your signed-out page, and `state` is dropped. That covers an unregistered address, a request that names no app, and a `client_id` that names a different app than the ID token. These requests used to get an error page. Now a valid ID token signs the person out in these cases too, where the error used to leave them signed in. A forged ID token is still refused. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #502](https://github.com/leptoskit/leptoskit/pull/502)

#### Commit only what the guest changed

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #543](https://github.com/leptoskit/leptoskit/pull/543)

A native app's commit no longer rebuilds the whole interface. The Wasm host records which nodes the guest changed since its last commit, and the renderer resolves and diffs only those parts, so updating one label in a 1,000-row list takes about 0.08 ms instead of 1.15 ms on an M-series Mac. Trees that use ids, `aria-labelledby` or `aria-describedby`, radio groups, or live regions still commit in full, because those resolve across the whole tree.

`NativeSurface::try_patch` is the new entry point for hosts that track their own changes.

- **Close live connections that exceed their frame budget.** A live WebSocket connection that sends frames faster than the server serves them now receives the rate-limit error and is then closed. Before, it got the error but stayed open. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #487](https://github.com/leptoskit/leptoskit/pull/487)
- **Deliver a live socket's last frames before it closes.** When the server closes a live WebSocket, such as one sending frames faster than it serves them, it now reads what the client is still sending until the client answers the close, for up to five seconds, before dropping the connection. Previously the connection could end in a reset that discarded the server's last frames, so on Windows a client lost the reason it was closed along with the close itself. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #497](https://github.com/leptoskit/leptoskit/pull/497)
- **Report missing desktop pedometer hardware.** The Apple pedometer plugin now answers a valid availability query with `false` on macOS and validates malformed requests. Permission and step-reading calls remain unavailable, and closing the plugin still cancels later calls. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #487](https://github.com/leptoskit/leptoskit/pull/487)

#### Bound SQLite memory and fix secure storage and clipboard

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #495](https://github.com/leptoskit/leptoskit/pull/495)

- The SQLite plugin stops a statement whose working memory grows by more than 64 MiB, and refuses temporary tables, views, indexes and triggers. Before, a `DISTINCT` over generated values, or temporary tables filled one call at a time, could exhaust the app's memory, because neither counted against the page quota.
- `SecureStorage` stores keys it hashes under a `sha256-` prefix. Before, the hex digest of a key such as `auth/token` was itself a readable key, so the two read and wrote the same secret.
- The Android clipboard plugin reads only an item's text. It no longer opens a content URI that another app placed on the clipboard, which read that app's provider on the UI thread with no size limit.

#### Launch the public website

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #523](https://github.com/leptoskit/leptoskit/pull/523)

`leptoskit-site` gains `--public-only`, the packaging mode behind <https://leptoskit.dev>. It produces the same polished landing page with its live Wasm component example, the complete documentation book under `/docs/`, and the configuration schema aliases at their stable `/schema.json` and `/schemas/` paths — as plain static files, with no console entrypoint, account API, or billing backend. The public page renders in a new public mode: navigation and calls to action lead to documentation and source, and the cloud section explains the CLI's deployment engine instead of offering the operator-enrolled pilot. The default packaging keeps building the complete hosted artifact with the console, so the pilot scripts and broker are unchanged; a reused output directory drops the previous console entrypoint when you switch to public-only.

The documentation book and the workspace now publish under the custom domain: the book's default URL and every brand and documentation link move from the old GitHub Pages host to `https://leptoskit.dev` (book pages under `/docs/`, schemas at their existing paths), and the docs workflow builds and deploys the real public-only artifact to GitHub Pages.

The live example's static button waits for the Wasm client to mount before accepting clicks, so the first interaction is not lost during loading.

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Packages:_ 🟢 _leptoskit_testing_library_, 🟢 _leptoskit_yoga_, 🟢 _leptoskit_use_, 🟢 _leptoskit_native_, 🟢 _leptoskit_testing_, 🟢 _leptoskit_tui_, 🟢 _leptoskit_appkit_, 🟢 _leptoskit_uikit_, 🟢 _leptoskit_android_, 🟢 _leptoskit_gtk_, 🟢 _leptoskit_windows_, 🟢 _leptoskit_aria_, 🟢 _leptoskit_analytics_, 🟢 _leptoskit_deploy_console_, 🟢 _leptoskit_form_, 🟢 _leptoskit_config_, 🟢 _leptoskit_plugin_, 🟢 _leptoskit_bridge_, 🟢 _leptoskit_bridge_ffi_, 🟢 _leptoskit_build_, 🟢 _leptoskit_plugin_crypto_, 🟢 _leptoskit_plugin_os_, 🟢 _leptoskit_plugin_clipboard_, 🟢 _leptoskit_plugin_haptics_, 🟢 _leptoskit_plugin_battery_, 🟢 _leptoskit_plugin_device_, 🟢 _leptoskit_plugin_secure_store_, 🟢 _leptoskit_plugin_linking_, 🟢 _leptoskit_plugin_passkeys_, 🟢 _leptoskit_plugin_web_auth_, 🟢 _leptoskit_plugin_filesystem_, 🟢 _leptoskit_plugin_network_, 🟢 _leptoskit_plugin_posture_, 🟢 _leptoskit_plugin_dialog_, 🟢 _leptoskit_plugin_document_picker_, 🟢 _leptoskit_plugin_screen_orientation_, 🟢 _leptoskit_plugin_light_sensor_, 🟢 _leptoskit_plugin_pedometer_, 🟢 _leptoskit_plugin_accelerometer_, 🟢 _leptoskit_plugin_application_, 🟢 _leptoskit_plugin_autostart_, 🟢 _leptoskit_plugin_barometer_, 🟢 _leptoskit_plugin_brightness_, 🟢 _leptoskit_plugin_gyroscope_, 🟢 _leptoskit_plugin_http_, 🟢 _leptoskit_plugin_keep_awake_, 🟢 _leptoskit_plugin_localization_, 🟢 _leptoskit_plugin_magnetometer_, 🟢 _leptoskit_plugin_permissions_, 🟢 _leptoskit_plugin_single_instance_, 🟢 _leptoskit_plugin_menu_, 🟢 _leptoskit_plugin_tray_, 🟢 _leptoskit_plugin_speech_, 🟢 _leptoskit_plugin_sqlite_, 🟢 _leptoskit_plugin_store_, 🟢 _leptoskit_plugin_image_manipulator_, 🟢 _leptoskit_plugin_screen_capture_, 🟢 _leptoskit_plugin_store_review_, 🟢 _leptoskit_plugin_sharing_, 🟢 _leptoskit_plugin_tracking_transparency_, 🟢 _leptoskit_plugin_websocket_, 🟢 _leptoskit_plugin_arguments_, 🟢 _leptoskit_plugin_shell_, 🟢 _leptoskit_plugin_logging_, 🟢 _leptoskit_plugin_notifications_, 🟢 _leptoskit_plugin_local_authentication_, 🟢 _leptoskit_plugin_location_, 🟢 _leptoskit_plugin_global_shortcut_, 🟢 _leptoskit_plugin_window_state_, 🟢 _leptoskit_sensor_, 🟢 _leptoskit_codepush_, 🟢 _leptoskit_wasm_, 🟢 _leptoskit_axum_, 🟢 _leptoskit_rpc_codegen_, 🟢 _leptoskit_rpc_macros_, 🟢 _leptoskit_rpc_, 🟢 _leptoskit_query_, 🟢 _leptoskit_virtual_, 🟢 _leptoskit_table_, 🟢 _leptoskit_rive_, 🟢 _leptoskit_rive_runtime_, 🟢 _leptoskit_docs_, 🟢 _leptoskit_macros_, 🟢 _leptoskit_router_macros_, 🟢 _leptoskit_router_, 🟢 _leptoskit_plugin_router_, 🟢 _leptoskit_router_codegen_, 🟢 _leptoskit_db_schema_, 🟢 _leptoskit_db_macros_, 🟢 _leptoskit_db_, 🟢 _leptoskit_auth_, 🟢 _leptoskit_idp_, 🟢 _leptoskit_extension_, 🟢 _leptoskit_devtools_protocol_, 🟢 _leptoskit_plugin_devtools_, 🟢 _leptoskit_devtools_, 🟢 _leptoskit_l10n_, 🟢 _leptoskit_l10n_codegen_, 🟢 _leptoskit_cli_, 🟢 _leptoskit_fmt_, 🟢 _leptoskit_lint_, 🟢 _leptoskit_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)

#### Fix a rounded-view tap crash; speed up list updates

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #490](https://github.com/leptoskit/leptoskit/pull/490)

Tapping a fully rounded view that clips its content, such as a `rounded-full overflow-hidden` avatar, could crash the app when the view sat at a fractional position. Hit testing on every platform, and the macOS `Pressable`, now handles that corner.

Lists and conditional views update faster in native apps:

- A `For` or `Show` that changes now rebuilds only its own children. It used to copy every node on the screen first, so toggling a `Show` beside 1,000 cards took 3 ms and now takes 4 µs.
- Removing many children at once costs their number rather than its square: clearing 1,000 cards takes about half as long.
- Two changes inside the same fixed-size cell, such as a label whose text and style both change, now lay out only that cell instead of the whole screen.
- On Linux, applying a batch of changes reconfigures the widget tree once per batch instead of once per change, so building a large screen no longer slows down with the square of its size.

#### Harden the Wasm runtime and code push against abuse

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #495](https://github.com/leptoskit/leptoskit/pull/495)

- A guest that imports the wasm-bindgen externref table shim with a reference-typed signature can no longer forge a function reference. Calling through one used to panic the interpreter, which aborted the macOS app.
- Plugin payload copies now cost fuel, so a guest looping over `submit` and `cancel` runs out of fuel instead of copying megabytes per instruction and freezing the app.
- Calls to the console-warn shim are logged at most 16 times per guest.
- Class strings longer than 4 KiB are parsed but no longer cached, so a guest can't pin hundreds of megabytes in the Tailwind parse cache.
- A synchronous Rust plugin that panics on the macOS UI thread now fails that call, and every later call to that plugin, with a native error instead of aborting the app.
- Code push reads `index.json` with a 2 MiB limit instead of 32 MiB, and `CodePushClient` caps rebuilt and downloaded containers at 32 MiB by default instead of 512 MiB. Custom `Transport`s can override the new `fetch_within` to stop reading at that limit.
- A signed release that the installed app can't run is remembered and isn't downloaded again on every check. A new store build forgets the list.
- iOS caches at most 256 colors and 64 fonts, so animating arbitrary colors or text sizes no longer grows memory without limit.

#### Close a channel bypass and bound live connections

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #496](https://github.com/leptoskit/leptoskit/pull/496)

A live connection could join any channel instance by naming its key, even though the server only checked whether the caller could join the instance its arguments described. Members of one room could then see another room's presence and signals and send signals into it, including across servers through the relay. The server now requires the key to be the join arguments it authorized. Two channels whose arguments look alike (`Chat { room }` and `Doc { room }`) no longer share a room.

Live connections are also bounded in more ways:

- 64 channels per connection.
- Join arguments of at most 4 KiB, and presence state of at most 16 KiB.
- At most 16 actions running at once.
- A resume repeats each subscription only once.
- Frames that don't parse count against the frame budget.
- A WebSocket message far over the 1 MiB frame limit closes the connection before it is buffered.
- A client that stops reading is disconnected after 30 seconds (`Live::stall_timeout`), instead of making the server hold everything meant for it.

Reusing a subscription ID for another query no longer returns the earlier query's result when the caller's context runs alone.

`rpc::paginate` reads at most `rpc::MAX_PAGE_ROWS` (1,000) rows per page and `rpc::MAX_PINNED_ROWS` per pinned page, whatever `limit` the client sends.

`leptoskit_axum`'s servers now close a connection that hasn't finished sending its request headers within `HEADER_READ_TIMEOUT` (30 seconds). Without that timeout, a few slow clients could hold every socket the server had.

#### Fix terminal wide text, resizes, and long lists

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #491](https://github.com/leptoskit/leptoskit/pull/491)

Terminal apps draw correctly in three cases they used to get wrong:

- Colored CJK text and emoji no longer overwrite each other. Each wide glyph now lands in the two cells it occupies.
- Resizing the terminal redraws everything on screen. Text that did not move used to stay blank until it changed.
- A scroll view whose content is taller than 32,767 rows, such as a long `VirtualList`, keeps showing its rows once scrolled.

A long scrolled list also renders faster: rows outside the screen are no longer wrapped or drawn. Rendering 10,000 rows into a 40×20 terminal takes 0.1 ms instead of 3 ms.

- **Blink f32 leftover-carry for flexible tracks.** Flexible grid tracks now distribute free space with binary32 flex shares, flooring each share to a 1/64 CSS-pixel layout unit and carrying the truncated fraction into the next track—matching Chrome’s `getBoundingClientRect` for equal and weighted `fr` rows/columns. Non-integral percentages such as `w-1/3` floor into a layout unit; whole-number percentages stay exact. _Packages:_ 🟢 _leptoskit_yoga_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #541](https://github.com/leptoskit/leptoskit/pull/541)
- **Yoga LayoutUnit rounding for Chrome grid parity.** Fractional grid tracks and percentage lengths resolve through Blink-style 1/64 CSS-pixel layout units, so Yoga frames match Chrome `getBoundingClientRect` for the shared Tailwind class trees. The grid parity probe covers nested grids, dense auto-flow, minmax, flex edges, absolute insets, percentages, and overflow. _Packages:_ 🟢 _leptoskit_yoga_, 🟢 _leptoskit_native_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #540](https://github.com/leptoskit/leptoskit/pull/540)
- **Paint italic text and per-edge borders on Linux and Android.** `border-t`, `border-l-2`, and the other edge border utilities now get the same default stroke color as a uniform `border`, and GTK and Android paint each edge instead of leaving those strokes undrawn. The `italic` utility also draws through GTK labels and Android text views, matching the web and the terminal. AppKit, UIKit, and Win32 still leave per-edge borders undrawn and italic upright. _Packages:_ 🟢 _leptoskit_native_, 🟢 _leptoskit_android_, 🟢 _leptoskit_gtk_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #534](https://github.com/leptoskit/leptoskit/pull/534)
- **Use Objective-C message forwarding for UIKit text input traits so debug builds can create text and password fields without crashing.** _Packages:_ 🟢 _leptoskit_uikit_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #516](https://github.com/leptoskit/leptoskit/pull/516)
- **GTK grid parity probe against Yoga frames.** Adds a `grid_parity_gtk` example that mounts the same Tailwind grid/flex fixtures as the web probe on real GTK 4 widgets under Xvfb, asserts allocations match Yoga within half a CSS pixel, and writes PNGs for cross-device layout evidence. _Packages:_ 🟢 _leptoskit_gtk_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #542](https://github.com/leptoskit/leptoskit/pull/542)
- **Reserve private operation logs before launching the deployment command so a repeated operation cannot execute again when its logs already exist.** _Packages:_ 🟢 _leptoskit_cli_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #516](https://github.com/leptoskit/leptoskit/pull/516)
- **Resolve deployment Git operations against the registered repository even when the CLI runs from a Git hook.** _Packages:_ 🟢 _leptoskit_cli_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #516](https://github.com/leptoskit/leptoskit/pull/516)
- **Explain how to connect to the deployment service when its saved credential is missing on Unix.** _Packages:_ 🟢 _leptoskit_cli_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #516](https://github.com/leptoskit/leptoskit/pull/516)
- **Paint italic text and per-edge borders on HarmonyOS.** The ArkUI host now applies `italic` and per-edge `border_widths` (`border-t`, `border-l-2`, …) the same way Android and GTK do, instead of leaving those strokes undrawn and italic upright. Deno render tests lock the Yoga frames from `grid_parity_probe` so absolute ArkUI placement stays aligned with the web HTML probe. _Packages:_ 🟢 _leptoskit_cli_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #539](https://github.com/leptoskit/leptoskit/pull/539)
- **Run recovery SQL without reading psql startup files, so local formatting settings cannot break backup verification.** _Packages:_ 🟢 _leptoskit_cli_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #516](https://github.com/leptoskit/leptoskit/pull/516)
