---
"release": minor
---

# `leptoskit publish` uploads the store screenshots

Each upload to the App Store, Google Play, and addons.mozilla.org now replaces the store's screenshots with the ones `leptoskit publish screenshots` saved in `store/screenshots/<size>/`, so a listing no longer needs them dragged in by hand.

- Name each size after the store size it is: `iphone-6.3`, `iphone-6.9`, `ipad-13`, and the other App Store sizes; `android-phone`, `android-tablet-7`, `android-tablet-10`, and `play-feature-graphic` for Google Play; `chrome-1280x800` for addons.mozilla.org. A folder leptoskit doesn't know is refused, by `leptoskit publish screenshots` too.
- Every image is checked first, with `--dry-run` as well: its size, how many there are, and transparency, which the App Store and Google Play refuse. Nothing is built or uploaded while one is wrong.
- Google Play's listing also gets the app's icon at 512×512, from `app.icon`.
- The Chrome Web Store and Edge have no API for images: their steps name the folder to upload from.
