---
"release": minor
---

# `app.icon`: every platform's icons from one image

Set `app.icon` in `leptoskit.toml` to one square PNG or SVG, 1024×1024 or larger, and every build makes the icons its platform needs: the iOS app icon (opaque, as the App Store requires), Android's launcher and adaptive icons for every density, the macOS `.icns`, and the browser extension's 16 to 128 pixel icons. `leptoskit publish` asks for an icon when a store needs one the app doesn't have. Generated Android projects add `.leptoskit/android/res` to their resources; older projects get a warning with the line to add.
