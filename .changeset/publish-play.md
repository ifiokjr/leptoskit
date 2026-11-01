---
"release": minor
---

# `leptoskit publish play` and Android App Bundles

`leptoskit publish play` uploads the Android app to Google Play. Name the track in `[publish.play]` (`track`, and `rollout` for a staged production release), and set a service account's key in `LEPTOSKIT_PUBLISH_PLAY_SERVICE_ACCOUNT`.

- An upload numbers the build one more than the highest version code on Google Play (or `--build-number <n>`), builds an App Bundle signed with the upload key, and puts it on the track as a draft. `--submit` releases it, or starts a staged rollout on production.
- Release builds make an App Bundle beside the APK, and generated Gradle scripts sign them with the upload key from `LEPTOSKIT_ANDROID_KEYSTORE`, `LEPTOSKIT_ANDROID_KEYSTORE_PASSWORD`, `LEPTOSKIT_ANDROID_KEY_ALIAS`, and `LEPTOSKIT_ANDROID_KEY_PASSWORD`, leaving them unsigned without. The app's `versionName` now comes from `app.version`.
- New projects target API 36, which Google Play requires of new apps and updates; `leptoskit publish play` checks the target, the signing, and the version code before it uploads.
