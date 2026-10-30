---
"release": minor
---

# Safari extensions inside the macOS and iOS apps

When `[extension].browsers` has `safari`, `leptoskit build --platform macos` and `--platform ios` package the extension for Safari and put it in the app, which is the only way Safari installs extensions. The macOS app gains `Contents/PlugIns/SafariExtension.appex`, signed with the App Sandbox entitlement Safari requires. The generated iOS Xcode project gains a `SafariExtension` app extension target, embedded in the app. The extension's bundle identifier is `native.package_id` plus `[extension.safari].bundle_id_suffix`, `SafariExtension` by default.
