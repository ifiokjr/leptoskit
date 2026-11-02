<h1 align="center">leptoskit_plugin_location</h1>

<p align="center">
  <strong>leptoskit_plugin_location</strong> is the leptoskit plugin for foreground location permission and cancellable bounded fixes through CoreLocation, Android LocationManager, or browser Geolocation.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_location"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_location"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_location.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_location"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_location/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

Foreground location acquisition using Apple CoreLocation, Android LocationManager, and browser Geolocation. Add this crate directly and select `location` in `native.plugins` for automatic registration. Apple supports iOS and macOS; there is no Windows/Linux location backend yet.

`permission` only queries. `request_permission(precise)` is a separate, intentional action; native reads never open a permission dialog. Android requests coarse and fine together when precise is requested, and accurately reports an approximate-only grant. iOS requests When In Use authorization and reports reduced/full accuracy without silently requesting temporary precise access. Browsers cannot expose the same accuracy permission distinction.

Apps own their permission declarations: iOS needs a nonempty `NSLocationWhenInUseUsageDescription`; macOS needs `NSLocationUsageDescription` and the location entitlement when sandboxed. The selected Android plugin declares coarse/fine permissions through its metadata; the runtime validates declarations before requesting access. No background permission is requested.

`current(Options::new(high_accuracy, timeout_ms, maximum_age_ms)?)` obtains a bounded fix. Timeouts are 1–120,000ms, cache tolerance is 0–3,600,000ms, and zero cache tolerance requires a fresh timestamp. High accuracy is a hint, not a guarantee of precision. `next(options, previous_timestamp_ms)` waits for a strictly newer fix; repeated calls provide a demand-driven foreground watch. One request is allowed at a time. Cancellation, completion, close, and native backgrounding stop updates, release listeners, and cancel timers. This is not a lossless path recorder, background tracker, geofencing API, or compass.

Android measures fix freshness with its monotonic elapsed-time clock, so changing the wall clock cannot make a stale cached fix appear fresh. The explicit `next` cursor remains a Unix timestamp on every platform; after a wall-clock rollback it may time out until timestamps exceed the cursor. Call `current` to establish a new cursor in that case.

Native reads recheck authorization before delivering coordinates. Revocation or a precise-to-approximate downgrade ends the pending read with `PermissionDenied`, so an already queued precise fix cannot escape under the reduced grant. Query permission and retry to start a new read under the current scope.

Fixes carry Unix milliseconds, latitude/longitude in degrees, horizontal accuracy in metres, and optional altitude, vertical accuracy, course, speed, provider, and simulated-source information. Unknown values remain `None`/Unknown. Vertical altitude datum and accuracy semantics remain platform-specific. Coordinates are never synthesized from zero values or unrelated data. `LocationError::TimedOut` is distinct from bridge permission/unavailable errors.

Browser operation requires a secure visible document. Reads first query Geolocation permission and reject any state other than granted. That query and acquisition are not atomic: revoking permission between them can still cause browser-controlled permission UI. An absolute no-prompt guarantee is unavailable on the web. Explicit `request_permission` uses a browser geolocation watch bounded to 30 seconds because browsers have no standalone permission-request API; the obtained fix is discarded. If that deadline expires, the request returns `Unavailable` without starting another permission query. Permission may still have changed, and callers can query it separately. Browser watches clear on drop and page hiding. Permission/Permissions-Policy denial is not bypassed.

The wire protocol uses methods 0 permission, 1 request permission, 2 current, and 3 newer fix. Options are three little-endian u32s: high-accuracy boolean, timeout, maximum age. Method 3 appends a finite nonnegative f64 previous timestamp. Permission replies are two u32s (status, accuracy scope). Acquisition replies start with u32 0 plus an 80-byte fix, or u32 1 for timeout. Fixes contain optional-presence bits, provider, simulated tri-state, a reserved zero u32, then eight f64s. Rust validates lengths, flags, finite values, coordinates, and optional-value ranges.

Tests use synthetic coordinates, mocked browser hardware, and codec fixtures. They do not request or read the developer's actual location. Physical GPS, OS permission prompts, precision reduction, and moving-device behavior need separate device acceptance testing.

References: [Apple CLLocationManager](https://developer.apple.com/documentation/corelocation/cllocationmanager), [Android LocationManager](https://developer.android.com/reference/android/location/LocationManager), [W3C Geolocation](https://www.w3.org/TR/geolocation/).

## How it fits

Add `leptoskit_plugin_location` as a direct dependency and select the `location` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Location` rather than resolving the plugin on every call. Native reads never prompt; request permission as a separate, intentional action, and the app supplies its usage description and manifest permissions.

## Use it

```toml
# In a native host, select `location` in `native.plugins`.
[dependencies]
leptoskit_plugin_location = { version = "0.1", features = ["guest"] }
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_location)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Plugin inventory](https://ifiokjr.github.io/leptoskit/platforms/plugin-inventory.html)
