# Runtime plugins

Runtime plugins give an installed LeptosKit host access to operating-system services. They are separate from `leptoskit_plugin`, whose hooks customize builds. Each runtime plugin is an independent Cargo crate. Adding a dependency does not grant its operations to a guest or install its Swift/Kotlin handlers.

## Current implementation

The [plugin inventory](plugin-inventory.md) records every current plugin crate, its implemented operations, platform coverage, registration path, and remaining Expo/Tauri/Electron scope. These are focused ports; no complete upstream API compatibility is claimed.

| Component                | Implemented behavior                                                                                                                                                                                 | Remaining work                                                                              |
| ------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------- |
| `leptoskit_bridge`       | Bounded binary calls/replies/events, host grants, cancellation, borrowed decoding, asynchronous interpreter calls and event receiver                                                                 | Per-plugin contracts and performance validation remain necessary                            |
| `leptoskit_wasm`         | Versioned plugin imports, guest memory validation, wake callback and guest executor polling                                                                                                          | Live guest replacement                                                                      |
| `leptoskit_bridge_ffi`   | C ABI, Swift and C++ ownership wrappers, Kotlin/JNI dispatch, retained Rust workers, native application lifecycle and controls                                                                       | Broader widget coverage and rendering benchmarks                                            |
| Native hosts             | Generated iOS, Android and macOS apps, selected-plugin registration, app directory context and cancellation-aware job accounting                                                                     | Windows/Linux application scaffolds and complete release packaging                          |
| Runtime crates           | Identity, device, battery/network state, text clipboard, haptics, links, permissions, secure storage, private files, settings, SQLite, localization, speech, wake locks, brightness and four sensors | See the operation-level [support matrix](plugin-inventory.md#current-plugin-support-matrix) |
| Direct host/browser APIs | Scoped HTTP/WebSocket clients and desktop/browser dialogs; explicitly configured autostart and host single-instance guard                                                                            | HTTP/WebSocket/dialog guest adapters and second-launch handoff                              |

Generated native apps boot a Rust/Wasm interface and register selected Rust, Swift and Kotlin plugins. See [native application projects](host-projects.md) for build commands and host-owned configuration. A compiled handler can still return `Unavailable` on a platform without the underlying capability: macOS sensor and brightness handlers do not imply working desktop sensors or brightness control.

## Registration and execution

The host's selected `native.plugins` list must match runtime metadata on direct Cargo dependencies. Guest clients normally enable their crate's `guest` feature and cache a resolved numeric plugin ID. The installed host declares methods and grants access explicitly; operating-system permissions are a separate decision.

Runtime metadata can name a Rust type (`rust-type`), an app-directory factory (`rust-factory`), an Apple module/type, or an Android handler. Shared native support packages are discovered through their `native-support` metadata. Android metadata can contribute manifest permissions and package-visibility queries. Apple purpose strings, entitlements and application-specific associations remain explicit host configuration; generators must not invent consent text.

`rust-worker = true` selects off-thread execution for a Rust handler. The generated registrant uses `app.install_worker`; filesystem, settings and SQLite use factories that retain app directories and defer disk access until a worker request. An owned worker retains the plugin and Rust payload without retaining the app or interpreter. Each plugin serializes its own invocations. Apple hosts use an `OperationQueue` with at most four active operations; blocking I/O does not run on Swift's cooperative executor. Android uses `Dispatchers.IO`. Replies return to the bridge's owning UI thread. Queued cancelled work is skipped; running synchronous I/O may finish and commit a side effect after the guest drops its future.

A synchronous Rust `Plugin::invoke` worker is not an asynchronous resource protocol. HTTP, WebSocket and dialog crates therefore expose direct APIs without claiming automatic guest registration. HTTP/WebSocket need a host-owned async runtime and cancellation-aware handles; dialogs need a live parent window and presentation lifecycle. Autostart requires host-selected identity/executable/arguments, and the single-instance guard must be acquired before constructing the app.

## Host and guest ownership

1. The guest resolves a plugin name once; subsequent calls use numeric IDs and raw bytes.
2. Submission queues work. It does not invoke UI code while the Wasm guest is on the stack. The host drains requests after the guest returns.
3. Swift handlers run on `MainActor`; Kotlin handlers run on the UI dispatcher. Slow work must suspend or use a worker.
4. The host completes replies on the bridge's owning thread and calls `WasmHost::wake_plugins`. The guest executor wakes waiting futures.
5. Dropping a queued call removes it. Dropping a running call queues a cancellation. Drain `Bridge::next_cancelled` (C/Swift: `lk_bridge_next_cancelled`, Kotlin: `NativeBridge.nextCancelled`) after guest entry points and cancel the corresponding native task. A bridge call slot remains occupied until the host takes that notification; the native job remains budgeted until it actually finishes.
6. Host close stops subscriptions and cancels jobs. Already-running OS operations may finish later, so their retained buffers and worker ownership must outlive that work. Their late replies must never reach a destroyed or replacement bridge.

Battery and network change observation use demand-driven waits for the next different snapshot. Intermediate states can coalesce. Each sensor request acquires one fresh reading and then stops acquisition; repeated calls are not a lossless fixed-rate recording stream. These contracts avoid an accumulating event backlog while preserving explicit cancellation. Keep-awake holds a pending request for each owned lease and releases it through the same cancellation path.

The interpreter also supports a single `guest::Events` receiver routing events by plugin and kind. Its asynchronous `recv()` shares the host wake callback with native-call futures. Dropping a pending wait preserves queued events; stopping native subscriptions remains a plugin operation. `test:plugins:guest` compiles a real Rust Wasm guest and checks call cancellation, permission denial, reply wakeups, receiver exclusivity and event delivery.

### Resource limits and errors

Bridge defaults are 128 outstanding calls, 256 buffered events, 8 MiB per payload and 32 MiB of buffered bytes. Unread replies and unobserved running-call cancellations retain call slots. Producers receive `Busy` when an event queue is full; adapters may explicitly define coalescing instead of silently dropping arbitrary events.

Taking a request transfers its buffer out of bridge accounting. Generated iOS, Android and macOS hosts independently limit retained native requests to 128 jobs and 32 MiB of request bytes, including queued and executing work. They reserve room for a maximum-size request before dequeuing, so small requests can pause conservatively when less than 8 MiB remains. Cancelling a task does not release its reservation; actual completion does. Queue drains are bounded to yield to the UI, and completion schedules another drain. These limits do not bound arbitrary allocations inside a plugin, the OS, or a reply under construction. Each plugin still needs its own limits.

Invalid Wasm pointers trap without consuming a reply. Oversized native replies become small terminal error replies. Duplicate and late completions are rejected. IDs are never reused within a bridge. Host grants prevent new calls after revocation; revoking an in-flight native operation requires cancellation. Permission denial, unsupported operations, invalid inputs, overload and native failures retain distinct statuses where the platform provides that distinction. Cancellation does not imply rollback of a write already in progress.

The Wasm protocol is documented in `crates/leptoskit_wasm/src/plugins.rs`; the native ABI is in `crates/leptoskit_bridge_ffi/include/leptoskit_bridge.h`. C pointers are a trusted embedding interface, not a boundary for untrusted guest pointers.

### Copies and performance evidence

The core has no global lock or mandatory async runtime. Call slots and queues are preallocated; arithmetic lookup avoids allocating a map entry per call. An allocator-instrumented test verifies zero heap allocations during 1,000 initialized Rust transport round trips. This is a narrow transport result, not a measurement of an OS plugin operation or rendering.

Rust payload ownership moves through `Bytes`; control-message decoding borrows fields. The Wasm-to-host boundary copies input once because guest linear memory can change after submission. Swift and C++ can retain a Rust buffer with explicit ownership. JNI exposes requests through a direct `ByteBuffer`. Kotlin reply arrays can require a JVM copy in addition to the Rust ownership copy. Large media needs a bounded stream or explicitly owned resource protocol; that protocol is not implied by the control bridge.

C++ handlers can return an existing Rust allocation through `lk_bridge_complete_owned` or `std::move(buffer).complete(bridge, call_id)`. Completion consumes the ownership handle even when the call was cancelled. Foreign-owned data uses the copying completion API. Tests verify that an owned completion preserves the payload address. Rust workers likewise retain Rust request/reply ownership across scheduling without translating through Swift `Data` or Kotlin byte arrays.

Plugin messages use binary payloads. Native rendering transactions still have their own JSON decoding path; the binary plugin bridge does not make rendering zero-copy. No end-to-end performance superiority over Flutter or React Native is claimed. Measure cold/warm calls, scheduling, native operations, allocations, large payloads, event delivery and UI-thread time separately on representative devices.

## Generate a project

```sh
leptoskit generate app my-app --platform web
leptoskit generate app my-app-native --platform native
leptoskit generate plugin my-plugin
```

Before the SDK is published, pass `--bridge-path /absolute/path/to/crates/leptoskit_bridge` to the plugin generator. It creates a standalone Cargo workspace with an echo method, registration metadata and an allocation-preservation test. Existing files are not overwritten. Custom Swift/Kotlin implementations must supply the corresponding native packages and handler metadata.

Native source validation includes SwiftLint and ktlint alongside Rust formatting and static analysis. Generated-app tests are necessary to verify that metadata discovery, native linking, lifecycle dispatch and guest replies work together; codec tests alone cannot establish that integration. Hardware output, permission prompts and platform policy still require appropriate device/browser evidence.

CI's `test:plugins:browser` runs the battery, dialog, filesystem, network, keep-awake, speech, settings, HTTP and shared sensor tests in Chrome, plus the WebSocket loopback-server runner. Some browser lifecycle cases use controlled API substitutes; those checks establish cleanup behavior rather than physical hardware support. Android's `native_plugin_suite` generates a host and exercises real platform handlers through instrumentation. Apple checks cover ownership, cancellation, worker scheduling and native builds; simulator app tests cover the generated host. These are complementary checks, not blanket upstream API parity.

## Ecosystem references and remaining scope

[Flutter's federated plugin model](https://docs.flutter.dev/packages-and-plugins/developing-packages) separates the app API from platform implementations. LeptosKit keeps distinct capability crates with explicit host grants. Workspace plugins follow lockstep releases; generated third-party crates have independent versions. [Flutter platform channels](https://docs.flutter.dev/platform-integration/platform-channels) inform asynchronous calls and events; LeptosKit uses binary payloads and numeric dispatch.

The [Expo SDK reference](https://docs.expo.dev/versions/latest/) and [Expo Modules API](https://docs.expo.dev/modules/overview/) provide the mobile capability inventory. [Tauri plugin development](https://v2.tauri.app/develop/plugins/) and [mobile plugins](https://v2.tauri.app/develop/plugins/develop-mobile/) illustrate Rust plus Swift/Kotlin packaging. Their entire inventories, with implemented subsets and exclusions, are tracked in the [port ledger](plugin-inventory.md).

[Electron's APIs](https://www.electronjs.org/docs/latest/api/app), [power monitoring](https://www.electronjs.org/docs/latest/api/power-monitor), [utility processes](https://www.electronjs.org/docs/latest/api/utility-process) and [performance guidance](https://www.electronjs.org/docs/latest/tutorial/performance) identify desktop lifecycle and tooling requirements. Remaining work includes windows/displays, menus/tray, shortcuts, notifications, inbound associations, second-launch handoff and native updates. These need LeptosKit window and lifecycle ownership rather than a hidden Electron/Tauri application.

[Electron Forge's build lifecycle](https://www.electronforge.io/core-concepts/build-lifecycle) separates packaging, installers and publication. LeptosKit now generates a runnable macOS `.app` in addition to iOS/Android hosts. Windows/Linux scaffolds, release installers, [signing](https://www.electronforge.io/guides/code-signing), notarization and publication are still separate work. [Forge configuration](https://www.electronforge.io/config/configuration) also underscores matching native artifacts to their runtime: LeptosKit must match Rust/Swift/Kotlin/C++ targets and bridge versions. Signed Wasm staging in `leptoskit_codepush` is not a native executable updater or a complete installed-app rollback system.
