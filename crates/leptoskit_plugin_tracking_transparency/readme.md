<h1 align="center">leptoskit_plugin_tracking_transparency</h1>

<p align="center">
  <strong>leptoskit_plugin_tracking_transparency</strong> is the leptoskit plugin for explicit iOS App Tracking Transparency status and authorization requests.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_tracking_transparency"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_tracking_transparency"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_tracking_transparency.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_tracking_transparency"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_tracking_transparency/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

Explicit iOS App Tracking Transparency (ATT) authorization. Query the current status or request a system consent UI following user intent. This port does not access advertising IDs, collect identifiers, install an analytics SDK, or start tracking.

Select `"tracking-transparency"` in `native.plugins` and add this crate to the app. Automatic registration supplies the owning Apple `NativePluginContext`. Put an accurate, app-owned `NSUserTrackingUsageDescription` string in `hosts/ios/Info.plist`; metadata declares this required key without supplying consent text. The runtime also rejects a missing, non-string or blank purpose before calling ATT. Optional localized descriptions and region-specific markdown descriptions remain app configuration.

The typed guest `TrackingTransparency` client exposes `status()` and `request()`. Status preserves `NotDetermined`, `Restricted`, `Denied` and `Authorized`. It is a current observation, not a permanent grant. Unknown platform statuses are errors. Status never prompts and does not cache an authorization decision.

Requests require an attached foreground scene and active application. ATT controls presentation and accepts no presenting controller; the explicit context establishes caller eligibility and lifecycle ownership. A request is always forwarded explicitly to ATT, including after prior denial or authorization: the OS determines regional eligibility, restrictions and whether a UI appears. Apple currently documents region-dependent re-request eligibility after a year. There is no fabricated `canPrompt` boolean or guarantee that a request presents a dialog. A dismissed prompt may leave status `NotDetermined`.

Across this plugin's instances, only one underlying ATT request is allowed process-wide. Overlap returns `Busy`. Cancellation, owning-scene background/disconnect, or plugin close ends the caller and removes its lifecycle observers. Apple provides no programmatic dismissal API, so the OS prompt may continue and its global slot remains occupied until Apple's completion arrives. A late result releases that slot without responding to a cancelled or closed caller. An old or duplicate callback cannot affect a newer request. Transient scene deactivation from the consent UI itself does not cancel the request. Returning foreground does not automatically prompt again.

Android, native desktop and web operations return `Unavailable`. Unsupported platforms never report synthetic authorization. Android registers an explicit unavailable adapter without adding advertising-ID permissions. A native Rust `Unsupported` handler is available for custom non-Apple hosts. Unlike Expo's broader module, advertising-ID APIs and Android advertising permissions are outside this port.

## Binary contract

Method `0` queries status and method `1` requests authorization. Both require exactly zero request bytes. Successful responses contain exactly one little-endian u32: `0` not determined, `1` restricted, `2` denied, `3` authorized. Trailing data and unknown enum values are rejected. Native calls use no JSON, polling, timer, or unbounded event queue.

## Validation

Tests use synthetic authorization providers and exercise denied/restricted/undecided states, foreground/configuration failures, overlap, cancellation, close/background cleanup, late and duplicate callbacks, and explicit re-request behavior. They do not display a real ATT prompt or access user identifiers. Device presentation and regional OS consent flows remain unexercised by these tests.

## References

- [Expo TrackingTransparency](https://docs.expo.dev/versions/latest/sdk/tracking-transparency/)
- [Apple requestTrackingAuthorization](https://developer.apple.com/documentation/apptrackingtransparency/attrackingmanager/requesttrackingauthorization(completionhandler:))
- [Apple NSUserTrackingUsageDescription](https://developer.apple.com/documentation/bundleresources/information-property-list/nsusertrackingusagedescription)

## How it fits

Add `leptoskit_plugin_tracking_transparency` as a direct dependency and select the `tracking-transparency` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::TrackingTransparency` rather than resolving the plugin on every call. It never reads advertising identifiers or starts tracking; iOS returns the system status, and other platforms return `Unavailable`.

## Use it

```toml
# In a native host, select `tracking-transparency` in `native.plugins`.
[dependencies]
leptoskit_plugin_tracking_transparency = { version = "0.1", features = ["guest"] }
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_tracking_transparency)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Plugin inventory](https://ifiokjr.github.io/leptoskit/platforms/plugin-inventory.html)
