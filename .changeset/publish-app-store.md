---
"release": minor
---

# `leptoskit publish app_store`: TestFlight and App Review

`leptoskit publish` now uploads iOS apps to App Store Connect. Name the app in `[publish.app_store]` (`app_id`, `team_id`, `testflight_groups`), and set an App Store Connect API key in `LEPTOSKIT_PUBLISH_APPLE_ISSUER_ID`, `LEPTOSKIT_PUBLISH_APPLE_KEY_ID`, and `LEPTOSKIT_PUBLISH_APPLE_KEY`.

- An upload archives the app with Xcode, signed automatically with the API key, uploads it, waits for processing, and adds it to the TestFlight groups. The build number is one more than the store's highest, or `--build-number <n>`.
- `--submit` creates or updates the App Store version, sets "What's New" from `store/en-US/release_notes.md`, attaches the build, and submits it for App Review.
- The steps after each run list what stays in App Store Connect: the listing, App Information, App Privacy, the age rating, export compliance, and App Review's contact.
- The generated iOS project now takes the app's version from `app.version`, and its build number from the build settings, for the app and its Safari extension.
