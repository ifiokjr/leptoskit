# Host projects: the Flutter lessons

leptoskit aims to be what Flutter is for its ecosystem: one `leptoskit` command that scaffolds, builds, runs, and ships an app on every platform. Flutter solved hard integration problems getting a foreign-runtime app into Xcode, Android Studio, and the desktop IDEs; this document is what we learned from studying it, what we adopted, and where leptoskit's design deliberately differs.

## How Flutter integrates with Xcode

Flutter's Xcode integration is the reference because it survives contact with real constraints — App Store review, code signing, CocoaPods-era plugins — and still keeps the developer out of Xcode almost entirely:

1. **The tool owns a generated layer, not the project.** Flutter writes `ios/Flutter/Generated.xcconfig` (paths, build settings) and the plugin registrant (`GeneratedPluginRegistrant.swift`) on every build. The checked-in `Runner.xcodeproj` never contains machine-specific state; anything generated is ephemeral and regenerable, so the project merges cleanly and works on every machine.
2. **Build phases delegate to the tool.** Xcode's build doesn't know about Dart; a build phase shells out to `flutter assemble`, which produces the artifacts (the App.framework, the Flutter engine framework) into known locations the project file references. The IDE stays the IDE; the tool stays the source of truth for compilation.
3. **`flutter attach` over a service protocol.** The app runs standalone; the CLI attaches for hot reload, logs, and the widget inspector. Nothing about the shipped app depends on the tool being connected.
4. **`flutter doctor` and `flutter devices`.** The first command anyone runs diagnoses the toolchain per platform; the second lists where an app can go next. Environment problems surface as actionable output, not build failures twenty minutes later.
5. **One `pubspec.yaml`.** Platform projects are an implementation detail the tool materializes; dependencies and assets are declared once, at the root, in the tool's format.

## What leptoskit adopts

| Flutter practice                         | leptoskit equivalent                                                                              | Status  |
| ---------------------------------------- | ------------------------------------------------------------------------------------------------- | ------- |
| Generated, regenerable project layer     | `leptoskit new` scaffolds; the CLI's build cache (`target/leptoskit/`) is the machine-local layer | done    |
| Tool-produced artifacts into known paths | `dist/pkg/` from the wasm pipeline; `.lkb` containers for releases                                | done    |
| `flutter doctor`                         | `leptoskit doctor` — cargo, rustc, wasm target, wasm-bindgen, wasm-opt, node                      | done    |
| One manifest for every platform          | `leptoskit.toml`: app identity, platforms, servers, channels, plugins                             | done    |
| Attach for hot reload                    | `leptoskit dev` watcher + cached pipeline; full attach protocol is on the roadmap                 | partial |
| `flutter devices`                        | planned (`leptoskit devices`) alongside the native host work                                      | planned |

The deeper shared design decision: **the store-shipped artifact is a thin host; the product ships over the air.** Flutter ships a Dart AOT snapshot in the store bundle; we ship the wasm interpreter and pull the interface as code push deltas. Their build must re-run for every store release; ours only for native-capability changes.

## Swift Packages, not CocoaPods

**leptoskit's Apple integration is Swift Package Manager first and only.** No Podfile, no podspec, no `pod install` step, ever. CocoaPods is in maintenance mode and Flutter itself is migrating off it; a new ecosystem in 2026 has no reason to carry it forward. What SPM gives the host:

- **The host is a package.** `hosts/apple/Package.swift` declares the app as an executable target with a resources bundle; Xcode opens the folder directly (`open hosts/apple`) with no project file to generate or corrupt — this is the modern replacement for Flutter's generated `.xcodeproj` management.
- **Rust links as a library target.** The runtime compiles to a static library plus a C header; the Swift package wraps it with a `systemLibrary`/binary target and the host is ordinary Swift calling ordinary imports. No build-phase shell-outs, no podspec.
- **`swift build` is the fallback toolchain.** CI can build the host with nothing but Swift itself.

`leptoskit new --platform native` scaffolds the package skeleton today (`hosts/apple/Package.swift` and a `main.swift` that marks where the runtime bridge lands); the runtime's C ABI is the remaining piece, tracked with the native backend work.

## What we deliberately do differently

- **The layout engine is ours.** Flutter's Skya/Impeller render everything; we map one-to-one onto platform widgets — a `leptoskit_native` element is a `UIView`/`NSView`/Android view, so the app inherits platform accessibility, input methods, and behavior for free.
- **The interface language is Rust**, compiled to wasm — no second language bridge at the boundary, and the same `Style` values and server functions the web target uses.
- **Axum is the server.** Flutter has no server story; leptoskit apps come with SSR, server functions, and the code push registry as one deployment surface.
