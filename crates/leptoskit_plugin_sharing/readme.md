# Sharing

Shares one UTF-8 text value (up to 64 KiB) or an explicitly validated HTTP(S) URL (up to 8192 bytes). The application supplies content; the user chooses the destination through iOS `UIActivityViewController`, macOS `NSSharingServicePicker`, Android's system chooser, or the browser Web Share API. No direct recipient or automatic sending API is exposed.

```rust,ignore
let content = leptoskit_plugin_sharing::Content::text("Hello from my app")?;
leptoskit_plugin_sharing::share(&content).await?;
```

Enable `guest` for native bridge calls. Select `sharing` in native configuration; Apple registration injects the owning `NativePluginContext`, and Android requires the generated activity's `NativeActivityResultHost`. No permissions, FileProvider, or entitlements are added. Custom hosts must provide these same presentation capabilities.

Browser applications call `web::share(&content)?.await?` directly inside a trusted user action. The call starts synchronously to preserve activation. HTTPS and Web Share support are required; `web::is_available()` checks these without opening UI. Browser policy and content restrictions may still reject a request. Dropping its future cannot close browser UI; the plugin retains busy admission until the browser promise settles.

An empty successful reply only means the platform completed its API callback. **It never confirms delivery.** Browser completion timing differs between operating systems. Android cannot reliably distinguish a dismissed chooser from choosing a target; either produces the same empty reply. iOS/macOS cancellations reported by their API return cancellation errors. A selected external service may continue showing compose UI after caller cancellation. Cancellation cannot retract content already handed to another service. macOS retains its busy slot until that service reports completion, unless the whole plugin closes.

Wire method 0 is a little-endian u32 content kind (0 text, 1 URL), then a u32 byte length and UTF-8 bytes; the reply is empty. All native adapters validate the wire before presenting UI. One presentation per plugin is admitted, and the owning host rejects competing Android external activities. Apple uses the explicit app controller, including a valid iPad popover anchor; it never searches for a global window.

Native URL parsers can reject a stricter subset than the Rust constructor. For example, Android rejects percent-encoded hostnames and abbreviated IPv4 addresses. Use ordinary ASCII or punycode hostnames and full IPv4 addresses for portable URLs.

This port does not share files, arbitrary filesystem paths, custom URL schemes, recipient addresses, or incoming share extensions. Windows/Linux native share integrations are not implemented; supported browsers on those platforms may expose Web Share. All automated tests use synthetic data and mocked completion. They never open a share destination or send content.

References: [Expo Sharing](https://docs.expo.dev/versions/latest/sdk/sharing/), [Android sharing](https://developer.android.com/training/sharing/send), [Apple activity UI](https://developer.apple.com/documentation/uikit/uiactivityviewcontroller), [Apple sharing picker](https://developer.apple.com/documentation/appkit/nssharingservicepicker), [Web Share](https://developer.mozilla.org/en-US/docs/Web/API/Navigator/share).
