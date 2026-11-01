<h1 align="center">leptoskit_plugin_store_review</h1>

<p align="center">
  <strong>leptoskit_plugin_store_review</strong> is the leptoskit plugin for owned-window in-app store review requests through StoreKit and Google Play.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_store_review"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_store_review"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_store_review.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_store_review"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_store_review/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

Requests an in-app review through StoreKit on iOS/macOS and Google Play on Android. Each request uses the application's supplied presentation context. The plugin never discovers a different application's scene or substitutes a global window.

Enable the `guest` feature for native Wasm clients. The cached client resolves the plugin once and uses numeric method IDs. Grant availability and request independently in the host policy. Native registration metadata supplies the Apple context and Android activity. Generated Android projects receive the pinned `com.google.android.play:review:2.0.2` dependency; its AAR supplies the required review activity.

| Platform        | Backend                                        | Request acknowledgment                      |
| --------------- | ---------------------------------------------- | ------------------------------------------- |
| iOS 17+         | `AppStore.requestReview(in: UIWindowScene)`    | StoreKit received the synchronous API call  |
| macOS 14+       | `AppStore.requestReview(in: NSViewController)` | StoreKit received the synchronous API call  |
| Android 26+     | Google Play `ReviewManager`                    | The review flow task completed successfully |
| Browser         | Explicitly unsupported                         | `web::request()` returns `Unavailable`      |
| Linux / Windows | No backend                                     | No native plugin registration               |

`Availability::Ready` means the backend has an active, owned presentation context. It does not reveal store eligibility, account state, installation source, rate limits or whether a prompt will appear. Google Play can reject preparation even after readiness was reported. StoreKit may suppress requests because of quotas, user preferences, TestFlight distribution or other platform policy.

Neither API reveals whether the dialog was displayed, a review was submitted, or a rating was selected. Do not turn `Accepted` into a “review completed” analytics event. StoreKit also provides no dismissal callback: its acknowledgment does not wait for the user to finish. Choose a natural point after a meaningful interaction, following the platform's review guidance, rather than gating an application action on a review.

Opening a store URL is a separate, explicit action through `leptoskit_plugin_linking`. This crate does not construct store URLs or redirect the user if in-app review is unavailable.

## Lifecycle

Apple requests require the supplied context's attached, active scene or visible macOS window. Querying availability never loads a detached view or requests UI. Invalidating the host context makes subsequent requests unavailable. Closing the plugin makes subsequent invocations cancelled. Once StoreKit receives a request, the plugin cannot retract its UI.

Android requires the supplied activity to be attached, focused and neither finishing nor destroyed. Only one flow is in flight per registered plugin. Cancellation during preparation prevents launching a late result. Cancellation after launch cancels the caller but preserves exclusive ownership until the Google Play task completes; another request returns `Busy` during that interval. Closing the plugin or destroying its activity cancels the waiting caller and prevents late presentation. The plugin cannot dismiss Google Play's own UI. Register one instance per host activity.

Store preparation failures return `Unavailable`; launch failures return `Native` with the underlying exception retained on Android. Cancelled operations report `Cancelled`. Applications should continue their normal workflow on errors.

## Protocol

Both methods require an empty payload. Responses are exactly one little-endian `u32`, with trailing bytes, truncated values and unknown tags rejected.

| Method       | ID | Response                                   |
| ------------ | -- | ------------------------------------------ |
| Availability | 0  | Unsupported=0, Ready=1, Inactive=2, Busy=3 |
| Request      | 1  | Accepted=1                                 |

The two-method protocol has no JSON serialization, user data, review text or rating field. Availability does not contact a store.

## Verification

Rust tests validate response domains and lengths. Swift tests inject a review service and verify owned-window readiness, invalidation, closure, malformed input and pre-cancelled tasks without calling StoreKit. Android instrumentation uses Google's `FakeReviewManager` with controlled completion tasks; it exercises real activity ownership and cancellation races without launching review UI.

References: [Expo StoreReview](https://docs.expo.dev/versions/latest/sdk/storereview/), [Apple StoreKit review controller](https://developer.apple.com/documentation/storekit/skstorereviewcontroller), [Apple AppStore review request](https://developer.apple.com/documentation/storekit/appstore/requestreview(in:)) and [Google Play review integration](https://developer.android.com/guide/playcore/in-app-review/kotlin-java).

## How it fits

Add `leptoskit_plugin_store_review` as a direct dependency and select the `store-review` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::StoreReview` rather than resolving the plugin on every call. Acceptance never proves a dialog appeared or a review was submitted, and the plugin never substitutes another application's scene.

## Use it

```rust,ignore
use leptoskit_plugin_store_review::{Availability, guest::StoreReview};

let review = StoreReview::new()?;
if review.availability().await? == Availability::Ready {
    let acceptance = review.request().await?;
    // Accepted acknowledges the platform call. It contains no review outcome.
}
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_store_review)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Plugin inventory](https://ifiokjr.github.io/leptoskit/platforms/plugin-inventory.html)
