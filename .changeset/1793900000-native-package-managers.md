---
"release": minor
---

# Native packages and a per-platform doctor

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
