---
"release": minor
---

# Plugin fixes from a line-by-line audit

- **passkeys:** Android now requires user verification on every passkey request, as iOS and macOS already did, whatever the options ask for.
- **location:** on Android, a second fix request while one is pending returns `Busy`, as the readme says. Before, it replaced the first request, whose location updates kept running and whose caller never got an answer.
- **network:** on iOS and macOS, `snapshot` works while `wait_for_change` is pending. It used to return `Busy`.
- **posture:**
  - On iOS before 27.1, `watch` waits instead of spinning.
  - A status read works while a watch is pending.
  - A fold outside the view no longer ends the watch with an invalid region.
  - On Android, closing the plugin ends a pending watch and its hinge listener.
- **screen-capture:** on iOS, opening Control Center to start a recording no longer ends a capture watch.
- **speech:** on iOS and macOS, a late cancel report from an earlier utterance can no longer stop the next one.
- **battery:** a new `State::NotCharging` reports a device that is plugged in but holding its charge. Android and macOS used to call this discharging or unknown.
- **haptics:** Android returns `Unavailable` when the system plays nothing, instead of success. Calls made off the main thread on iOS and macOS return `Native` instead of `InvalidArgument`.
- **localization:**
  - Android reports modern language codes (`he`, `id`) that match the locale tag, not legacy ones (`iw`, `in`).
  - On desktop, an unknown time zone no longer fails the whole snapshot.
- **device:** an Android phone in a desk dock is no longer reported as a desktop, and VR headsets report `Headset`.
- **notifications:** iOS and macOS no longer ask for sound permission, which the plugin never uses.
- **clipboard:** the experimental Rust Android path reads plain text only, like the Kotlin plugin, instead of opening content URIs on the UI thread.
- **web-auth:** on desktop, a call with the wrong session ID no longer discards the real session.
- **http:** requests are limited to 128 headers, the same limit the bridge carries, and browser responses with more are refused the same way.
- Readmes for secure-store, shell, web-auth and menu, and the plugin matrix, now describe what the code does: macOS keychain accessibility, process-group containment, cancellation per platform, Windows test coverage, and shell's automatic registration on macOS.
