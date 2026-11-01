# Store review

Requests an in-app review through StoreKit on iOS/macOS and Google Play on Android. Each request uses the application's supplied presentation context. The plugin never discovers a different application's scene or substitutes a global window.

```rust,ignore
use leptoskit_plugin_store_review::{Availability, guest::StoreReview};

let review = StoreReview::new()?;

if review.availability().await? == Availability::Ready {
    let acceptance = review.request().await?;
    // Accepted acknowledges the platform call. It contains no review outcome.
}
```

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
