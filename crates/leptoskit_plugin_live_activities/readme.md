<h1 align="center">leptoskit_plugin_live_activities</h1>

<p align="center">
  <strong>leptoskit_plugin_live_activities</strong> is the leptoskit plugin for starting, updating, and ending iOS Live Activities, including Dynamic Island presentations, with optional ActivityKit push-token hooks.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_live_activities"><strong>Docs</strong></a> ·
  <a href="https://github.com/leptoskit/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_plugin_live_activities"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_live_activities"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_live_activities.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_live_activities"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_live_activities/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://img.shields.io/github/actions/workflow/status/leptoskit/leptoskit/ci.yml?label=CI"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

Live Activities keep a short-lived status on the Lock Screen and, on supported devices, in the Dynamic Island while the user is outside the app. This plugin starts, updates, and ends those activities through ActivityKit, and can wait for the per-activity push token Apple uses for remote updates.

| Platform        | Implementation                         | Scope                                                                                          |
| --------------- | -------------------------------------- | ---------------------------------------------------------------------------------------------- |
| iOS 16.1+       | Swift ActivityKit                      | Enablement queries, start/update/end, push-token wait, tracked activity list                   |
| iPadOS          | Same ActivityKit path where available  | Lock Screen presentations; Dynamic Island only on devices that have one                        |
| macOS           | Unavailable                            | ActivityKit Live Activities are not offered                                                    |
| Android         | Kotlin stub                            | Every method returns `Unavailable`                                                             |
| Browser / other | Rust `web` / `Unsupported`             | Every method returns `Unavailable`                                                             |

A successful `start` means ActivityKit accepted the request. It does not prove Lock Screen or Dynamic Island chrome appeared. The app must include a Widget Extension that renders `LeptosKitLiveActivityAttributes` from this package. Custom attribute types, push-to-start tokens, remote payload authoring, and App Intent deep links from the island are out of scope for this port.

## Entitlements and ActivityKit requirements

Apps own their host configuration. The plugin does not invent Info.plist keys or capabilities:

1. Set `NSSupportsLiveActivities` to `true` in the iOS app's `Info.plist`.
2. Optionally set `NSSupportsLiveActivitiesFrequentUpdates` to `true` when the product needs frequent remote updates; `frequent_updates_enabled` still reports the user's current system setting.
3. For push updates (`push_updates: true` on start, then `wait_push_token`), enable the Push Notifications capability and a valid APNs environment. Remote payloads must follow Apple's ActivityKit push format and target the token this plugin returns.
4. Add a Widget Extension target that imports `LeptosKitLiveActivities` and provides SwiftUI views for `LeptosKitLiveActivityAttributes` / `ContentState`.
5. Live Activities require a real device or a simulator OS that supports ActivityKit; Focus modes and Low Power Mode can still suppress presentation after a successful start.

On every non-iOS platform the installed handler validates payloads where applicable and returns `Unavailable`. Cross-platform apps should treat that as "no Live Activities here", not as a user denial.

## Native registration and use

Select `"live-activities"` in `native.plugins` and add this crate to the app. Automatic registration supplies the Apple and Android types from Cargo metadata. Enable the `guest` feature in Wasm guest code and grant the methods the app needs in the host policy. The guest client caches its numeric plugin ID.

Content state is a bounded map of string fields (at most 16 keys). Keys follow the same identifier rules as activity IDs. The Widget Extension decides how those fields appear on the Lock Screen and Dynamic Island.

## Binary protocol

All integers are little-endian `u32`. Strings are `u32` byte lengths followed by strict UTF-8. Decoders reject trailing bytes, truncated fields, invalid UTF-8 and values outside their domain before calling ActivityKit.

| ID | Method                     | Request                                                                 | Response                |
| -- | -------------------------- | ----------------------------------------------------------------------- | ----------------------- |
| 0  | Are enabled                | Empty                                                                   | Flag `u32`              |
| 1  | Frequent updates enabled   | Empty                                                                   | Flag `u32`              |
| 2  | Start                      | id, kind, title, push flag, field count, key/value pairs                | id string               |
| 3  | Update                     | id, field count, key/value pairs                                        | Empty                   |
| 4  | End                        | id, dismissal kind, after-seconds, optional final fields                | Empty                   |
| 5  | Wait push token           | id                                                                      | length-prefixed bytes   |
| 6  | List                       | Empty                                                                   | count, id strings       |

IDs and field keys contain 1–64 ASCII letters, digits, `.`, `_` or `-`. Titles have at most 128 UTF-8 bytes and must contain non-whitespace text. Field values have at most 256 UTF-8 bytes. Control characters are rejected. Push tokens are 1–512 raw bytes. Dismissal is Default=0, Immediate=1 or After=2 with `after-seconds` in 1..=86400 (zero for Default/Immediate). Flags are exactly 0 or 1.

## Verification and references

Rust tests cover codecs, bounds and the unsupported host stub. Apple tests inject a fake ActivityKit service to exercise enablement, lifecycle, push-token delivery and missing-backend unavailability without posting system Live Activities.

API scope is informed by [Apple Live Activities](https://developer.apple.com/documentation/activitykit) and [Updating Live Activities with push notifications](https://developer.apple.com/documentation/activitykit/updating-and-ending-your-live-activity-with-activitykit-push-notifications).

## How it fits

<!-- {=p_live_activitiesSummary} -->

Add `leptoskit_plugin_live_activities` as a direct dependency and select the `live-activities` plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata. Generated Windows, Linux, and terminal hosts currently omit runtime plugins; use explicit registration in a custom host there. Guests enable the `guest` feature and reuse `guest::LiveActivities` rather than resolving the plugin on every call. Only iOS implements ActivityKit; other platforms return `Unavailable`. Starting an activity does not prove Lock Screen or Dynamic Island UI appeared: the app must ship a Widget Extension that renders `LeptosKitLiveActivityAttributes`.

<!-- {/p_live_activitiesSummary} -->

## Use it

```rust,ignore
use leptoskit_plugin_live_activities::{
    ContentState, DismissalPolicy, EndRequest, Identifier, StartRequest, UpdateRequest,
};
use leptoskit_plugin_live_activities::guest::LiveActivities;

let client = LiveActivities::new()?;
if !client.are_enabled().await? {
    return Ok(());
}

let id = Identifier::new("ride-1")?;
client
    .start(&StartRequest::new(
        id.clone(),
        Identifier::new("ride")?,
        "Driver en route",
        ContentState::new([("eta", "4"), ("driver", "Alex")])?,
        true,
    )?)
    .await?;

// Optional: send the token to your server for ActivityKit push updates.
let token = client.wait_push_token(&id).await?;
let _hex = token.to_hex();

client
    .update(&UpdateRequest::new(
        id.clone(),
        ContentState::new([("eta", "2"), ("driver", "Alex")])?,
    ))
    .await?;

client
    .end(&EndRequest::new(
        id,
        DismissalPolicy::After { seconds: 30 },
        Some(ContentState::new([("eta", "0"), ("status", "arrived")])?),
    ))
    .await?;
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_live_activities)
- [leptoskit documentation](https://leptoskit.dev/docs/)
- [Repository](https://github.com/leptoskit/leptoskit)
- [Plugin inventory](https://leptoskit.dev/docs/platforms/plugin-inventory.html)
