# Shared crate summaries

Edit these blocks, then run `devenv shell docs:update`. The README and Rust crate docs use the same text.

<!-- {@kitSummary} -->

Start here. A web build enables `web` and renders through leptos; a native build enables a
platform backend and commits the same tree as real widgets. Features add routing, queries,
forms, localization, database, auth, extensions, and `DevTools` only when the app asks for
them, so the meta crate stays a front door rather than a bundle of everything.

<!-- {/kitSummary} -->

<!-- {@agentUiSummary} -->

It interprets interfaces an agent writes as text while the app runs: Markdown with
components, variables, formulas, charts, and actions. The documents are data, never
code, so a published app shows new interfaces without a new build.

<!-- {/agentUiSummary} -->

<!-- {@analyticsSummary} -->

See which application routes are failing or slow. The app keeps bounded minute aggregates
and a small correlation buffer in memory. Connect the optional deployment service for
durable fleet history, or use the per-process endpoint without an extra observability
service. Unexported in-memory data resets on restart.

<!-- {/analyticsSummary} -->

<!-- {@androidSummary} -->

Compile `android/WidgetHost.kt` into an Android app and construct `AndroidBackend::new` on
the UI thread. Every built-in primitive renders a real view; callbacks propose typed
events that the app commits through `NativeSurface`.

<!-- {/androidSummary} -->

<!-- {@appSummary} -->

One `LeptosKit` component tree manages hosted installations from the web and native
iOS/Android clients. The accompanying `leptoskit-cloud` binary hosts accounts, Stripe
billing, per-account deployment journals, encrypted `OpenTofu` state and the worker API.
Customer code runs on a dedicated outbound worker, using the existing CLI engine.

<!-- {/appSummary} -->

<!-- {@appkitSummary} -->

A `NativeSurface` built on it renders the same tree the web build renders as DOM. AppKit
is the reference backend, used by the gallery and custom Rust hosts. Generated macOS apps
use the smaller Swift renderer in `leptoskit_bridge_ffi`.

<!-- {/appkitSummary} -->

<!-- {@ariaSummary} -->

It owns behavior, not pixels: app code styles the pieces with Tailwind and gets shared
focus, keyboard, and ARIA metadata. Platform renderers determine which semantics reach
assistive technology. `leptoskit_form` builds its fields on top of it.

<!-- {/ariaSummary} -->

<!-- {@authSummary} -->

It owns the server-side session and the shared client, and uses the passkey and web-auth
plugins for native sign-in. `leptoskit_idp` turns the same accounts into an OIDC provider.

<!-- {/authSummary} -->

<!-- {@axumSummary} -->

The generated server mounts it beside `leptos_axum`; apps that serve their own assets use
the same functions. It stays leptos-free so the server binary only pays for what it uses.

<!-- {/axumSummary} -->

<!-- {@bridgeSummary} -->

Plugins define methods, payload limits, cancellation, and error codes once; the guest and
the Swift, Kotlin, or C++ host then speak the same protocol. Most apps never name it
directly, but every runtime plugin does.

<!-- {/bridgeSummary} -->

<!-- {@bridge_ffiSummary} -->

Generated macOS, iOS, Android, and HarmonyOS shells link against it; the Rust backends are
used by tests and by hosts that embed leptoskit directly. It is the boundary between the
platform's UI thread and the guest.

<!-- {/bridge_ffiSummary} -->

<!-- {@buildSummary} -->

Generated apps call it from `build.rs`. It lets a shared module be replaced by a
`.macos.rs`, `.ios.rs`, or `.web.rs` file without changing the imports in the rest of
the app.

<!-- {/buildSummary} -->

<!-- {@canvasSummary} -->

It is the drawing model for custom graphics that native widgets cannot express: a
`Scene` lists shapes, paths, gradients, shadows, clips, and layers in order, and the
wgpu renderer draws it on the GPU. It is standalone for now; later work hosts it in
native surfaces and the browser.

<!-- {/canvasSummary} -->

<!-- {@cliSummary} -->

`leptoskit verify build` builds a backend from a public Git commit in a container without
network access, twice, and records a bundle, its inventory, and SLSA provenance;
`leptoskit verify reproduce` lets anyone rebuild that bundle and compare the source,
dependencies, and executable byte for byte. `verify record` and `verify check` inventory
and check a bundle offline. None of them verifies a running backend. See [checking a
backend bundle](https://leptoskit.dev/docs/guide/30-verifying-releases.html). Every workflow starts
here. The binary is a thin wrapper over the library, so the same code drives
`leptoskit dev`, CI builds, native host preparation, and release checks. Generated
projects call it by name; tooling calls the library directly.

<!-- {/cliSummary} -->

<!-- {@codepushSummary} -->

`leptoskit codepush` uses it to pack and publish, `leptoskit_axum` serves it, and
`leptoskit_wasm` consumes it on the device. It is the React Native update model applied to
the interface bundle.

<!-- {/codepushSummary} -->

<!-- {@configSummary} -->

The CLI, build pipeline, dev server, and native hosts all parse the same configuration, so
they agree on what the app is. Unknown keys and invalid combinations fail with the file
and field before a build starts.

<!-- {/configSummary} -->

<!-- {@dbSummary} -->

It gives shared code a typed database without hand-written SQL; `leptoskit db` diffs the
schema and writes migrations. Auth and IDP build their schema packs on it.

<!-- {/dbSummary} -->

<!-- {@db_macrosSummary} -->

Apps use them through `leptoskit::db` or `leptoskit_db`; this crate is the macro
implementation. Every mistake it can catch is a compile error that says what to write
instead.

<!-- {/db_macrosSummary} -->

<!-- {@db_schemaSummary} -->

`leptoskit db` and the `leptoskit_db` runtime use it; apps never depend on it. It uses no database connection or files, and renders Postgres and SQLite SQL from the same plan,
with each step's risk assessed against existing rows.

<!-- {/db_schemaSummary} -->

<!-- {@deploy_consoleSummary} -->

One optional service hosts the dashboard, durable operation queue, scoped agent API and
analytics history. It uses SQLite and reuses the existing `OpenTofu`/Kamal engine through
fixed CLI operations. No Redis or separate monitoring database is required.

<!-- {/deploy_consoleSummary} -->

<!-- {@devtoolsSummary} -->

Under `leptoskit dev`, the CLI starts the hub and these agents open sessions, mirror
trees, and carry commands back. Hooks expose the route, named state, forms, and query
caches that only the app knows.

<!-- {/devtoolsSummary} -->

<!-- {@devtools_protocolSummary} -->

It has no platform code: the browser, native host, guest, and front end all speak the same
protocol. A version mismatch is refused and both versions are named.

<!-- {/devtools_protocolSummary} -->

<!-- {@docsSummary} -->

The leptoskit book is built with it, and apps can use the same generator for their own
docs. It is MDX-shaped but Rust-native: components are leptos components, and mistakes
fail the build with a location.

<!-- {/docsSummary} -->

<!-- {@extensionSummary} -->

Apps enable the `extension` feature, name entry points with `entry!`, and build with
`--platform extension`. APIs return `Unavailable` instead of throwing, and listeners are
removed when their handle drops.

<!-- {/extensionSummary} -->

<!-- {@fmtSummary} -->

rustfmt leaves `view!` markup alone, so this fills the gap without a second Rust
formatter. It runs in editor integrations, pre-commit, and CI, and agrees with the lint
rules.

<!-- {/fmtSummary} -->

<!-- {@formSummary} -->

A form declares `FieldSpec` s once; the same state drives web inputs and native controls,
and `leptoskit_aria` supplies the accessible field wiring. Validation and submission logic
stay out of the view layer.

<!-- {/formSummary} -->

<!-- {@gtkSummary} -->

Initialize GTK on the application thread, construct `GtkBackend`, and hand it to
`NativeSurface`. `open_window` hosts a committed root and `run_application` drives the
loop; GTK objects stay on their creating thread.

<!-- {/gtkSummary} -->

<!-- {@idpSummary} -->

It turns the accounts `leptoskit_auth` already manages into an identity provider, without
a second user store. Authorization, consent, and token issuance stay in the app.

<!-- {/idpSummary} -->

<!-- {@l10nSummary} -->

Messages live in one ARB file per locale; `leptoskit l10n` writes the typed module, and
app code calls getters instead of parsing ARB at runtime. It works in SSR, the browser,
and native guests.

<!-- {/l10nSummary} -->

<!-- {@l10n_codegenSummary} -->

The `leptoskit l10n` command and the CLI's build pipeline use it; apps never depend on it.
It checks every translation against the template and reports placeholder, plural, and
naming problems with a file and line.

<!-- {/l10n_codegenSummary} -->

<!-- {@lintSummary} -->

rustc and clippy check Rust; these rules check the conventions a leptoskit project relies
on, with diagnostics that name the file, line, and fix.

<!-- {/lintSummary} -->

<!-- {@macrosSummary} -->

Use them through `leptoskit::prelude`. The macro parses rstml markup and emits builder
calls, so markup and builder code produce one element tree and neither is a second-class
path.

<!-- {/macrosSummary} -->

<!-- {@nativeSummary} -->

Every native backend implements `NativeBackend` and consumes the mutations this crate
produces; the web build uses its optional `web` feature to render the same builders
through leptos. App code normally reaches it through `leptoskit::prelude`, not as a
direct dependency, so one screen works on every target.

<!-- {/nativeSummary} -->

<!-- {@pluginSummary} -->

A plugin crate implements `LeptoskitPlugin` and is declared in `leptoskit.toml`; leptos
plugins are forwarded to the app build as cargo features. Runtime native plugins use the
bridge instead; this is the build-time half.

<!-- {/pluginSummary} -->

<!-- {@p_accelerometerSummary} -->

Add `{{ plugin_accelerometer.package.name }}` as a direct dependency and select the `{{ plugin_accelerometer.package.metadata.leptoskit.runtime.name }}`
plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata.
Generated Windows, Linux, and terminal hosts currently omit runtime plugins; use explicit
registration in a custom host there. Guests enable the `guest` feature and reuse
`guest::Accelerometer` rather than resolving the plugin on every call. iOS and Android use
real system sensors, desktop has no sensor, and the browser uses Generic Sensor or
`DeviceMotion`.

<!-- {/p_accelerometerSummary} -->

<!-- {@p_applicationSummary} -->

Add `{{ plugin_application.package.name }}` as a direct dependency and select the `{{ plugin_application.package.metadata.leptoskit.runtime.name }}`
plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata.
Generated Windows, Linux, and terminal hosts currently omit runtime plugins; use explicit
registration in a custom host there. Guests enable the `guest` feature and reuse
`guest::Application` rather than resolving the plugin on every call. Apple reads
`Bundle.main`, Android reads the running package record, and a Rust desktop host supplies
its own packaging metadata; browsers have no equivalent installed-app identity.

<!-- {/p_applicationSummary} -->

<!-- {@p_argumentsSummary} -->

Add `leptoskit_plugin_arguments` as a direct dependency and expose the plugin explicitly
from the native host; it has no `native.plugins` entry. Guests enable the `guest` feature
and reuse `guest::ArgumentsClient` rather than resolving the plugin on every call. The
host chooses which arguments to disclose; mobile and browser process capture returns
`Unavailable`, while explicit snapshots and parsing still work.

<!-- {/p_argumentsSummary} -->

<!-- {@p_autostartSummary} -->

Native hosts construct it explicitly and may expose it through the bridge to
`guest::AutostartClient`. There is no automatic `native.plugins` registration.
Constructing the manager never enables login startup; registration is an explicit call,
and mobile, browser, and unsupported desktops return `Unavailable`.

<!-- {/p_autostartSummary} -->

<!-- {@p_barometerSummary} -->

Add `{{ plugin_barometer.package.name }}` as a direct dependency and select the `{{ plugin_barometer.package.metadata.leptoskit.runtime.name }}` plugin
in `native.plugins`; supported generated hosts discover it from Cargo metadata. Generated
Windows, Linux, and terminal hosts currently omit runtime plugins; use explicit
registration in a custom host there. Guests enable the `guest` feature and reuse
`guest::Barometer` rather than resolving the plugin on every call. iOS and Android use
real pressure sensors, desktop has no sensor, and no relative altitude estimate is
fabricated.

<!-- {/p_barometerSummary} -->

<!-- {@p_batterySummary} -->

Add `{{ plugin_battery.package.name }}` as a direct dependency and select the `{{ plugin_battery.package.metadata.leptoskit.runtime.name }}` plugin in
`native.plugins`; supported generated hosts discover it from Cargo metadata. Generated
Windows, Linux, and terminal hosts currently omit runtime plugins; use explicit
registration in a custom host there. Guests enable the `guest` feature and reuse
`guest::Battery` rather than resolving the plugin on every call. Unknown readings stay
unknown instead of being reported as a full battery, and a simulator is never presented as
physical hardware.

<!-- {/p_batterySummary} -->

<!-- {@p_brightnessSummary} -->

Add `{{ plugin_brightness.package.name }}` as a direct dependency and select the `{{ plugin_brightness.package.metadata.leptoskit.runtime.name }}`
plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata.
Generated Windows, Linux, and terminal hosts currently omit runtime plugins; use explicit
registration in a custom host there. Guests enable the `guest` feature and reuse
`guest::Brightness` rather than resolving the plugin on every call. Android changes only
the activity window, iOS changes the screen and restores the previous value, and desktop
and the browser return `Unavailable`.

<!-- {/p_brightnessSummary} -->

<!-- {@p_clipboardSummary} -->

Add `{{ plugin_clipboard.package.name }}` as a direct dependency and select the `{{ plugin_clipboard.package.metadata.leptoskit.runtime.name }}` plugin
in `native.plugins`; supported generated hosts discover it from Cargo metadata. Generated
Windows, Linux, and terminal hosts currently omit runtime plugins; use explicit
registration in a custom host there. Guests enable the `guest` feature and reuse
`guest::Clipboard` rather than resolving the plugin on every call. Only plain text is
supported; browser operations need a secure context and may need a user gesture.

<!-- {/p_clipboardSummary} -->

<!-- {@p_cryptoSummary} -->

Add `{{ plugin_crypto.package.name }}` as a direct dependency and select the `{{ plugin_crypto.package.metadata.leptoskit.runtime.name }}` plugin in
`native.plugins`; supported generated hosts discover it from Cargo metadata. Generated
Windows, Linux, and terminal hosts currently omit runtime plugins; use explicit
registration in a custom host there. Native hashing and entropy run on the Rust worker,
and browsers use the same Rust implementation compiled to Wasm.

<!-- {/p_cryptoSummary} -->

<!-- {@p_deviceSummary} -->

Add `{{ plugin_device.package.name }}` as a direct dependency and select the `{{ plugin_device.package.metadata.leptoskit.runtime.name }}` plugin in
`native.plugins`; supported generated hosts discover it from Cargo metadata. Generated
Windows, Linux, and terminal hosts currently omit runtime plugins; use explicit
registration in a custom host there. Guests enable the `guest` feature and reuse
`guest::Device` rather than resolving the plugin on every call. Native guests, Rust
desktop hosts, and browsers each return a validated binary snapshot with the fields that
platform can actually provide.

<!-- {/p_deviceSummary} -->

<!-- {@p_devtoolsSummary} -->

Apps never select it. The shared runtime installs it in `leptoskit dev` builds, and the
host merges the guest's route, state, forms, and queries into the session it reports.

<!-- {/p_devtoolsSummary} -->

<!-- {@p_dialogSummary} -->

Use this crate directly from a Rust desktop host or its browser adapter. It has no runtime
plugin metadata, guest client, or `native.plugins` registration; mobile document import
uses `leptoskit_plugin_document_picker`. Desktop calls need an initialized application
event loop and a parent window; the browser adapter uses the File System Access API with
an explicit byte limit.

<!-- {/p_dialogSummary} -->

<!-- {@p_document_pickerSummary} -->

Add `{{ plugin_document_picker.package.name }}` as a direct dependency and select the
`{{ plugin_document_picker.package.metadata.leptoskit.runtime.name }}` plugin in `native.plugins`; supported generated hosts discover it from
Cargo metadata. Generated Windows, Linux, and terminal hosts currently omit runtime
plugins; use explicit registration in a custom host there. The user picks the document;
the app receives owned bytes under a size limit and never gets a reusable path into the
document provider.

<!-- {/p_document_pickerSummary} -->

<!-- {@p_filesystemSummary} -->

Add `{{ plugin_filesystem.package.name }}` as a direct dependency and select the `{{ plugin_filesystem.package.metadata.leptoskit.runtime.name }}`
plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata.
Generated Windows, Linux, and terminal hosts currently omit runtime plugins; use explicit
registration in a custom host there. Guests enable the `guest` feature and reuse
`guest::Filesystem` rather than resolving the plugin on every call. Every native path
stays inside the application-owned data directory; guests cannot select an arbitrary
filesystem root.

<!-- {/p_filesystemSummary} -->

<!-- {@p_global_shortcutSummary} -->

Native hosts construct `GlobalShortcuts` after event-loop startup and may expose
`ShortcutPlugin` to the typed guest client. There is no automatic `native.plugins`
registration. macOS, Windows, and X11 are supported; native Wayland, browsers, and mobile
return `Unavailable`, with no `XWayland` fallback or fake browser shortcut.

<!-- {/p_global_shortcutSummary} -->

<!-- {@p_gyroscopeSummary} -->

Add `{{ plugin_gyroscope.package.name }}` as a direct dependency and select the `{{ plugin_gyroscope.package.metadata.leptoskit.runtime.name }}` plugin
in `native.plugins`; supported generated hosts discover it from Cargo metadata. Generated
Windows, Linux, and terminal hosts currently omit runtime plugins; use explicit
registration in a custom host there. Guests enable the `guest` feature and reuse
`guest::Gyroscope` rather than resolving the plugin on every call. iOS and Android use
real system sensors, desktop has no sensor, and the browser converts `DeviceMotion` degrees
to radians.

<!-- {/p_gyroscopeSummary} -->

<!-- {@p_hapticsSummary} -->

Add `{{ plugin_haptics.package.name }}` as a direct dependency and select the `{{ plugin_haptics.package.metadata.leptoskit.runtime.name }}` plugin in
`native.plugins`; supported generated hosts discover it from Cargo metadata. Generated
Windows, Linux, and terminal hosts currently omit runtime plugins; use explicit
registration in a custom host there. Guests enable the `guest` feature and reuse
`guest::Haptics` rather than resolving the plugin on every call. iOS uses UIKit feedback
generators, Android uses platform haptic feedback, and macOS approximates the mobile
styles with its trackpad performer; browsers use the Vibration API where allowed.

<!-- {/p_hapticsSummary} -->

<!-- {@p_httpSummary} -->

Add `{{ plugin_http.package.name }}` as a direct dependency and select the `{{ plugin_http.package.metadata.leptoskit.runtime.name }}` plugin in
`native.plugins`; supported generated hosts discover it from Cargo metadata. Generated
Windows, Linux, and terminal hosts currently omit runtime plugins; use explicit
registration in a custom host there. Native requests use reqwest with rustls and need a
Tokio runtime; the browser uses fetch with incremental body reading. Redirects are not
followed, and every destination origin is explicit.

<!-- {/p_httpSummary} -->

<!-- {@p_image_manipulatorSummary} -->

Add `{{ plugin_image_manipulator.package.name }}` as a direct dependency and select the
`{{ plugin_image_manipulator.package.metadata.leptoskit.runtime.name }}` plugin in `native.plugins`; supported generated hosts discover it
from Cargo metadata. Generated Windows, Linux, and terminal hosts currently omit runtime
plugins; use explicit registration in a custom host there. Native calls run on the Rust
worker so a batch takes one bridge round trip; browser calls use the same engine compiled
to Wasm, and large jobs belong in a Web Worker.

<!-- {/p_image_manipulatorSummary} -->

<!-- {@p_keep_awakeSummary} -->

Add `{{ plugin_keep_awake.package.name }}` as a direct dependency and select the `{{ plugin_keep_awake.package.metadata.leptoskit.runtime.name }}`
plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata.
Generated Windows, Linux, and terminal hosts currently omit runtime plugins; use explicit
registration in a custom host there. Guests enable the `guest` feature and reuse
`guest::KeepAwake` rather than resolving the plugin on every call. iOS preserves and
restores the idle timer, Android uses the activity's keep-screen-on flag, desktop uses OS
assertions, and the browser uses the Screen Wake Lock API; none of them promise background
execution.

<!-- {/p_keep_awakeSummary} -->

<!-- {@p_light_sensorSummary} -->

Add `{{ plugin_light_sensor.package.name }}` as a direct dependency and select the `{{ plugin_light_sensor.package.metadata.leptoskit.runtime.name }}`
plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata.
Generated Windows, Linux, and terminal hosts currently omit runtime plugins; use explicit
registration in a custom host there. Guests enable the `guest` feature and reuse
`guest::LightSensor` rather than resolving the plugin on every call. iOS, macOS, and
desktop have no supported public ambient-light API and return `Unavailable`; display
brightness is never substituted for illumination.

<!-- {/p_light_sensorSummary} -->

<!-- {@p_linkingSummary} -->

Add `{{ plugin_linking.package.name }}` as a direct dependency and select the `{{ plugin_linking.package.metadata.leptoskit.runtime.name }}` plugin in
`native.plugins`; supported generated hosts discover it from Cargo metadata. Generated
Windows, Linux, and terminal hosts currently omit runtime plugins; use explicit
registration in a custom host there. Guests enable the `guest` feature and reuse
`guest::Linking` rather than resolving the plugin on every call. Incoming deep links need
host lifecycle integration and are not handled by this crate.

<!-- {/p_linkingSummary} -->

<!-- {@p_local_authenticationSummary} -->

Add `{{ plugin_local_authentication.package.name }}` as a direct dependency and select the
`{{ plugin_local_authentication.package.metadata.leptoskit.runtime.name }}` plugin in `native.plugins`; supported generated hosts discover it
from Cargo metadata. Generated Windows, Linux, and terminal hosts currently omit runtime
plugins; use explicit registration in a custom host there. Guests enable the `guest`
feature and reuse `guest::LocalAuthentication` rather than resolving the plugin on every
call. This is not account identity or server authorization; the plugin reports explicit
terminal outcomes and never invents a success.

<!-- {/p_local_authenticationSummary} -->

<!-- {@p_localizationSummary} -->

Add `{{ plugin_localization.package.name }}` as a direct dependency and select the `{{ plugin_localization.package.metadata.leptoskit.runtime.name }}`
plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata.
Generated Windows, Linux, and terminal hosts currently omit runtime plugins; use explicit
registration in a custom host there. Guests enable the `guest` feature and reuse
`guest::Localization` rather than resolving the plugin on every call. Each snapshot
reflects current platform state, no global locale is mutated, and a host should re-query
after the application resumes.

<!-- {/p_localizationSummary} -->

<!-- {@p_locationSummary} -->

Add `{{ plugin_location.package.name }}` as a direct dependency and select the `{{ plugin_location.package.metadata.leptoskit.runtime.name }}` plugin in
`native.plugins`; supported generated hosts discover it from Cargo metadata. Generated
Windows, Linux, and terminal hosts currently omit runtime plugins; use explicit
registration in a custom host there. Guests enable the `guest` feature and reuse
`guest::Location` rather than resolving the plugin on every call. Native reads never
prompt; request permission as a separate, intentional action, and the app supplies its
usage description and manifest permissions.

<!-- {/p_locationSummary} -->

<!-- {@p_loggingSummary} -->

Add `{{ plugin_logging.package.name }}` as a direct dependency and select the `{{ plugin_logging.package.metadata.leptoskit.runtime.name }}` plugin in
`native.plugins`; supported generated hosts discover it from Cargo metadata. Generated
Windows, Linux, and terminal hosts currently omit runtime plugins; use explicit
registration in a custom host there. There is no global logger or remote backend; the host
chooses the log directory and limits, and the guest protocol stays bounded independently
of JSON-lines output.

<!-- {/p_loggingSummary} -->

<!-- {@p_magnetometerSummary} -->

Add `{{ plugin_magnetometer.package.name }}` as a direct dependency and select the `{{ plugin_magnetometer.package.metadata.leptoskit.runtime.name }}`
plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata.
Generated Windows, Linux, and terminal hosts currently omit runtime plugins; use explicit
registration in a custom host there. Guests enable the `guest` feature and reuse
`guest::Magnetometer` rather than resolving the plugin on every call. This is a magnetic
sensor measurement, not a compass heading; platform calibration differs, and desktop has
no sensor.

<!-- {/p_magnetometerSummary} -->

<!-- {@p_menuSummary} -->

Native hosts construct it explicitly; it is not a guest plugin and installs nothing by
default. The desktop host owns the menu and installs it on its own windows; definitions
are validated before any native allocation, and mobile and the browser expose no menu.

<!-- {/p_menuSummary} -->

<!-- {@p_networkSummary} -->

Add `{{ plugin_network.package.name }}` as a direct dependency and select the `{{ plugin_network.package.metadata.leptoskit.runtime.name }}` plugin in
`native.plugins`; supported generated hosts discover it from Cargo metadata. Generated
Windows, Linux, and terminal hosts currently omit runtime plugins; use explicit
registration in a custom host there. Guests enable the `guest` feature and reuse
`guest::Network` rather than resolving the plugin on every call. Apple uses
`NWPathMonitor`, Android uses `ConnectivityManager`, desktop reports interface state,
and the browser uses `navigator.onLine` plus `NetworkInformation`; an active connection
never claims a server is reachable.

<!-- {/p_networkSummary} -->

<!-- {@p_notificationsSummary} -->

Add `{{ plugin_notifications.package.name }}` as a direct dependency and select the `{{ plugin_notifications.package.metadata.leptoskit.runtime.name }}`
plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata.
Generated Windows, Linux, and terminal hosts currently omit runtime plugins; use explicit
registration in a custom host there. Guests enable the `guest` feature and reuse
`guest::Notifications` rather than resolving the plugin on every call. A successful call
means the OS accepted the request, not that a banner appeared; focus modes and user
settings can still suppress it.

<!-- {/p_notificationsSummary} -->

<!-- {@p_osSummary} -->

Add `{{ plugin_os.package.name }}` as a direct dependency and select the `{{ plugin_os.package.metadata.leptoskit.runtime.name }}` plugin in
`native.plugins`; supported generated hosts discover it from Cargo metadata. Generated
Windows, Linux, and terminal hosts currently omit runtime plugins; use explicit
registration in a custom host there. The values come from compile-time constants of the
host build, so the guest sees the real platform even though its own target is wasm32.

<!-- {/p_osSummary} -->

<!-- {@p_passkeysSummary} -->

Add `{{ plugin_passkeys.package.name }}` as a direct dependency and select the `{{ plugin_passkeys.package.metadata.leptoskit.runtime.name }}` plugin in
`native.plugins`; supported generated hosts discover it from Cargo metadata. Generated
Windows, Linux, and terminal hosts currently omit runtime plugins; use explicit
registration in a custom host there. Guests enable the `guest` feature and reuse
`guest::Passkeys` rather than resolving the plugin on every call. Apple platforms use
`ASAuthorization` and Android uses Credential Manager; WebAuthn JSON crosses the bridge
unchanged.

<!-- {/p_passkeysSummary} -->

<!-- {@p_pedometerSummary} -->

Add `{{ plugin_pedometer.package.name }}` as a direct dependency and select the `{{ plugin_pedometer.package.metadata.leptoskit.runtime.name }}` plugin
in `native.plugins`; supported generated hosts discover it from Cargo metadata. Generated
Windows, Linux, and terminal hosts currently omit runtime plugins; use explicit
registration in a custom host there. Guests enable the `guest` feature and reuse
`guest::Pedometer` rather than resolving the plugin on every call. iOS uses `CoreMotion` and
Android uses its step counter; the app supplies the motion usage description or
activity-recognition permission.

<!-- {/p_pedometerSummary} -->

<!-- {@p_permissionsSummary} -->

Add `{{ plugin_permissions.package.name }}` as a direct dependency and select the `{{ plugin_permissions.package.metadata.leptoskit.runtime.name }}`
plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata.
Generated Windows, Linux, and terminal hosts currently omit runtime plugins; use explicit
registration in a custom host there. Guests enable the `guest` feature and reuse
`guest::Permissions` rather than resolving the plugin on every call. Querying status never
shows a prompt; request only after the user chooses, and use the dedicated location,
notification, and photo APIs for permissions with different OS flows.

<!-- {/p_permissionsSummary} -->

<!-- {@p_postureSummary} -->

Add `{{ plugin_posture.package.name }}` as a direct dependency and select the `{{ plugin_posture.package.metadata.leptoskit.runtime.name }}` plugin in
`native.plugins`; supported generated hosts discover it from Cargo metadata. Generated
Windows, Linux, and terminal hosts currently omit runtime plugins; use explicit
registration in a custom host there. Guests enable the `guest` feature and reuse
`guest::DevicePosture` rather than resolving the plugin on every call. A posture describes
the device the app is running on and every hinge that crosses its window; desktop reports
a rigid device; supporting browsers use Device Posture and Viewport Segments, with a rigid
fallback when those APIs are absent.

<!-- {/p_postureSummary} -->

<!-- {@p_routerSummary} -->

Generated apps enable it by default, so `src/routes/` is always in step with the build.
The runtime and derive live in `leptoskit_router`; this crate only owns the build hook.

<!-- {/p_routerSummary} -->

<!-- {@p_screen_captureSummary} -->

Add `{{ plugin_screen_capture.package.name }}` as a direct dependency and select the
`{{ plugin_screen_capture.package.metadata.leptoskit.runtime.name }}` plugin in `native.plugins`; supported generated hosts discover it from
Cargo metadata. Generated Windows, Linux, and terminal hosts currently omit runtime
plugins; use explicit registration in a custom host there. Guests enable the `guest`
feature and reuse `guest::ScreenCapture` rather than resolving the plugin on every call.
The plugin never captures pixels or reads screenshots; Android can protect its owned
window, while iOS can observe scene capture and app screenshots without prevention.

<!-- {/p_screen_captureSummary} -->

<!-- {@p_screen_orientationSummary} -->

Add `{{ plugin_screen_orientation.package.name }}` as a direct dependency and select the
`{{ plugin_screen_orientation.package.metadata.leptoskit.runtime.name }}` plugin in `native.plugins`; supported generated hosts discover it
from Cargo metadata. Generated Windows, Linux, and terminal hosts currently omit runtime
plugins; use explicit registration in a custom host there. Guests enable the `guest`
feature and reuse `guest::ScreenOrientation` rather than resolving the plugin on every
call. A pending hold is not an acknowledgement that the OS rotated; the snapshot reports
the presented interface, not accelerometer orientation.

<!-- {/p_screen_orientationSummary} -->

<!-- {@p_secure_storeSummary} -->

Add `{{ plugin_secure_store.package.name }}` as a direct dependency and select the `{{ plugin_secure_store.package.metadata.leptoskit.runtime.name }}`
plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata.
Generated Windows, Linux, and terminal hosts currently omit runtime plugins; use explicit
registration in a custom host there. Guests enable the `guest` feature and reuse
`guest::SecureStore` rather than resolving the plugin on every call. Native mobile
handlers ship with the crate; desktop hosts construct the store over their own credential
backend, and secrets never become a guest-selected filesystem path.

<!-- {/p_secure_storeSummary} -->

<!-- {@p_sharingSummary} -->

Add `{{ plugin_sharing.package.name }}` as a direct dependency and select the `{{ plugin_sharing.package.metadata.leptoskit.runtime.name }}` plugin in
`native.plugins`; supported generated hosts discover it from Cargo metadata. Generated
Windows, Linux, and terminal hosts currently omit runtime plugins; use explicit
registration in a custom host there. The app supplies the content, the user picks the
destination, and completion only means the platform finished its API callback.

<!-- {/p_sharingSummary} -->

<!-- {@p_sensorsSummary} -->

Add `{{ plugin_sensors.package.name }}` as a direct dependency and select the `{{ plugin_sensors.package.metadata.leptoskit.runtime.name }}` plugin
in `native.plugins`; supported generated hosts discover it from Cargo metadata. Generated
Windows, Linux, and terminal hosts currently omit runtime plugins; use explicit registration
in a custom host there. Guests enable the `guest` feature and reuse `guest::Sensors`, which
streams the accelerometer, gravity-free acceleration, gyroscope, magnetometer, and barometer
from one plugin. iOS and Android use real system sensors on the same axes, desktop has no
sensor, and the browser uses Generic Sensor or `DeviceMotion` and has no barometer.

<!-- {/p_sensorsSummary} -->

<!-- {@p_shellSummary} -->

Generated desktop hosts select `{{ plugin_shell.package.metadata.leptoskit.runtime.name }}` in `native.plugins` with explicit command policies;
guests enable `guest` and use `guest::ShellClient`. Custom hosts can construct and expose
it explicitly. Generated Windows, Linux, and terminal hosts currently omit runtime
plugins, so they require custom registration. macOS, Linux, and Windows run only absolute
executables with fixed argument slots and an explicit environment; the host never accepts
a shell command string from a guest.

<!-- {/p_shellSummary} -->

<!-- {@p_single_instanceSummary} -->

Native hosts construct it explicitly; it is not a guest plugin and installs nothing by
default. Call `InstanceGuard::acquire` before constructing the app and keep the guard
alive; macOS, Linux, and Windows release the OS lock on drop or process exit, including a
crash.

<!-- {/p_single_instanceSummary} -->

<!-- {@p_speechSummary} -->

Add `{{ plugin_speech.package.name }}` as a direct dependency and select the `{{ plugin_speech.package.metadata.leptoskit.runtime.name }}` plugin in
`native.plugins`; supported generated hosts discover it from Cargo metadata. Generated
Windows, Linux, and terminal hosts currently omit runtime plugins; use explicit
registration in a custom host there. Guests enable the `guest` feature and reuse
`guest::Speech` rather than resolving the plugin on every call. iOS and macOS use
`AVSpeechSynthesizer`, Android uses `TextToSpeech`, and the browser uses
`speechSynthesis`; overlapping speech returns `Busy`.

<!-- {/p_speechSummary} -->

<!-- {@p_sqliteSummary} -->

Add `{{ plugin_sqlite.package.name }}` as a direct dependency and select the `{{ plugin_sqlite.package.metadata.leptoskit.runtime.name }}` plugin in
`native.plugins`; supported generated hosts discover it from Cargo metadata. Generated
Windows, Linux, and terminal hosts currently omit runtime plugins; use explicit
registration in a custom host there. Guests enable the `guest` feature and reuse
`guest::Sqlite` rather than resolving the plugin on every call. The database lives in
Rust, not a platform channel or JavaScript engine, with native implementations for iOS,
Android, macOS, Windows, and Linux. Generated Windows and Linux hosts do not yet register
runtime plugins; custom hosts must register it themselves. The browser has no SQLite
plugin.

<!-- {/p_sqliteSummary} -->

<!-- {@p_storeSummary} -->

Add `{{ plugin_store.package.name }}` as a direct dependency and select the `{{ plugin_store.package.metadata.leptoskit.runtime.name }}` plugin in
`native.plugins`; supported generated hosts discover it from Cargo metadata. Generated
Windows, Linux, and terminal hosts currently omit runtime plugins; use explicit
registration in a custom host there. Guests enable the `guest` feature and reuse
`guest::Store` rather than resolving the plugin on every call. Each application gets one
store, values are JSON over a binary envelope, and guests cannot choose filesystem paths.

<!-- {/p_storeSummary} -->

<!-- {@p_store_reviewSummary} -->

Add `{{ plugin_store_review.package.name }}` as a direct dependency and select the `{{ plugin_store_review.package.metadata.leptoskit.runtime.name }}`
plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata.
Generated Windows, Linux, and terminal hosts currently omit runtime plugins; use explicit
registration in a custom host there. Guests enable the `guest` feature and reuse
`guest::StoreReview` rather than resolving the plugin on every call. Acceptance never
proves a dialog appeared or a review was submitted, and the plugin never substitutes
another application's scene.

<!-- {/p_store_reviewSummary} -->

<!-- {@p_tracking_transparencySummary} -->

Add `{{ plugin_tracking_transparency.package.name }}` as a direct dependency and select the
`{{ plugin_tracking_transparency.package.metadata.leptoskit.runtime.name }}` plugin in `native.plugins`; supported generated hosts discover it
from Cargo metadata. Generated Windows, Linux, and terminal hosts currently omit runtime
plugins; use explicit registration in a custom host there. Guests enable the `guest`
feature and reuse `guest::TrackingTransparency` rather than resolving the plugin on every
call. It never reads advertising identifiers or starts tracking; iOS returns the system
status, and other platforms return `Unavailable`.

<!-- {/p_tracking_transparencySummary} -->

<!-- {@p_traySummary} -->

Native hosts construct it explicitly; it is not a guest plugin and installs nothing by
default. The host calls `TrayHost::install` after its event loop starts and owns removal;
Linux uses the GTK-free KSNI backend, and mobile and the browser have no tray.

<!-- {/p_traySummary} -->

<!-- {@p_web_authSummary} -->

Add `{{ plugin_web_auth.package.name }}` as a direct dependency and select the `{{ plugin_web_auth.package.metadata.leptoskit.runtime.name }}` plugin in
`native.plugins`; supported generated hosts discover it from Cargo metadata. Generated
Windows, Linux, and terminal hosts currently omit runtime plugins; use explicit
registration in a custom host there. Guests enable the `guest` feature and reuse
`guest::WebAuth` rather than resolving the plugin on every call. It follows RFC 8252: the
provider page never loads in a web view inside the app.

<!-- {/p_web_authSummary} -->

<!-- {@p_websocketSummary} -->

Add `{{ plugin_websocket.package.name }}` as a direct dependency and select the `{{ plugin_websocket.package.metadata.leptoskit.runtime.name }}` plugin
in `native.plugins`; supported generated hosts discover it from Cargo metadata. Generated
Windows, Linux, and terminal hosts currently omit runtime plugins; use explicit
registration in a custom host there. Native connections use tokio-tungstenite with rustls
and need a Tokio runtime; browser connections own their socket and event callbacks. Every
destination origin is explicit.

<!-- {/p_websocketSummary} -->

<!-- {@p_window_stateSummary} -->

Native hosts construct it explicitly; it is not a guest plugin and installs nothing by
default. The portable state and placement API works everywhere; native capture and restore
adapters currently ship for macOS, and the host owns persistence and which window is
restored.

<!-- {/p_window_stateSummary} -->

<!-- {@querySummary} -->

It is the data layer for screens that read server functions: queries share a cache across
components, refetch on focus and reconnect, persist across tabs or native windows, and
pause offline. `leptoskit_rpc` supplies the calls it wraps.

<!-- {/querySummary} -->

<!-- {@riveSummary} -->

The shared `Rive` component uses it on every backend that supports vector playback; the
runtime crate pins the offline C++ build behind it. Apps bring their own assets and keep
playback out of the network path.

<!-- {/riveSummary} -->

<!-- {@rive_runtimeSummary} -->

Applications use `leptoskit_rive`; this crate exposes no application API. Its separate
Cargo build lets changes to the Rust player or image adapter reuse the C++ compilation
cache.

<!-- {/rive_runtimeSummary} -->

<!-- {@routerSummary} -->

The CLI's `leptoskit routes` runs the codegen; `leptoskit_plugin_router` runs it before
cargo; this crate is what app code imports. A route file is the only place a URL shape is
declared, so links and parsing cannot drift.

<!-- {/routerSummary} -->

<!-- {@router_codegenSummary} -->

The `leptoskit routes` command and the CLI's build pipeline use it; apps never depend on
it. It reports every problem with its file and line, and never overwrites a file leptoskit
did not write.

<!-- {/router_codegenSummary} -->

<!-- {@router_macrosSummary} -->

It is an implementation detail of `leptoskit_router`, which re-exports the derive.
`Option<T>` may be absent, `Vec<T>` takes every value, and other types are required unless
the field is `#[route(default)]`.

<!-- {/router_macrosSummary} -->

<!-- {@rpcSummary} -->

It is the boundary between shared screens and the server. The same call runs during SSR,
in the browser, and in a native guest; `leptoskit_rpc_codegen` turns the specification
into TypeScript, Python, Go, Rust, Dart, and CLI clients.

<!-- {/rpcSummary} -->

<!-- {@rpc_codegenSummary} -->

The server writes the specification; the CLI reads it back and generates clients that
speak the RPC's own protocol, plus an OpenAPI 3.1 document. The generators sit behind a
default feature, so the server binary can depend on the types alone.

<!-- {/rpc_codegenSummary} -->

<!-- {@rpc_macrosSummary} -->

Use them through `leptoskit_rpc`; an `#[api]` publishes a leptos server function to
generated clients, while query and mutation attributes generate operation arguments and
server registration. Retry and cache behavior belongs to `leptoskit_rpc` and
`leptoskit_query`.

<!-- {/rpc_macrosSummary} -->

<!-- {@sensorSummary} -->

The sensor plugins share its request/reply layout, cancellation rules, timestamps, and
browser fallbacks, so an app can switch sensors without relearning the contract. Apps
depend on the individual sensor crate; this crate is the common floor.

<!-- {/sensorSummary} -->

<!-- {@tableSummary} -->

It computes the model; the app renders rows with leptoskit components and keeps the same
behavior on web and native. It pairs with `leptoskit_virtual` for long lists.

<!-- {/tableSummary} -->

<!-- {@testingSummary} -->

It tests element trees without a device, using semantic finders, interactions, bounded
pumping, and diagnostic trees. The CLI can run the same journeys against a real native
app.

<!-- {/testingSummary} -->

<!-- {@testing_librarySummary} -->

It mounts through the web renderer and shares finders with `leptoskit_testing`, so a team
can test the browser build with the same vocabulary it uses for native widget tests.

<!-- {/testing_librarySummary} -->

<!-- {@tuiSummary} -->

Every cell is 8×16 logical pixels, so Yoga layout, Tailwind spacing, and text measurement
land on the monospace grid. `leptoskit dev/build --platform tui` drives it, and
`examples/opencode` is a full coding-agent interface built with it.

<!-- {/tuiSummary} -->

<!-- {@uikitSummary} -->

It consumes the same Yoga mutation stream as every other backend. Custom Rust hosts embed
it through `UIKitHost`. Generated iOS apps use the smaller Swift renderer in
`leptoskit_bridge_ffi`.

<!-- {/uikitSummary} -->

<!-- {@useSummary} -->

The hooks read leptos signals and accept host services explicitly, so the same code runs
in a browser and in a native guest. It is the utility layer beneath app screens, not a UI
framework.

<!-- {/useSummary} -->

<!-- {@virtualSummary} -->

It decides which items a viewport intersects so a long list only builds the visible rows,
on web and native alike. Infinite queries and tables use the same model.

<!-- {/virtualSummary} -->

<!-- {@wasmSummary} -->

This is what makes the shipped app a shell: the interface arrives as Wasm, renders through
the same renderer, and can be replaced without a store update. The generated iOS, Android,
macOS, Windows, and Linux hosts embed it.

<!-- {/wasmSummary} -->

<!-- {@windowsSummary} -->

The backend owns its HWNDs on one UI thread and consumes the existing Yoga mutation
stream; it never embeds a browser or creates a second layout tree. The executable includes
the provided manifest for version 6 common controls and layered-window support.

<!-- {/windowsSummary} -->

<!-- {@yogaSummary} -->

`leptoskit_native` calls it for every commit; backends only apply the frames it returns.
It is also usable on its own when a Rust tool needs React Native-style layout or a CSS
grid implementation without a browser.

<!-- {/yogaSummary} -->
