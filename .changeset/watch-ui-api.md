---
"release": minor
---

# Watch UI API for Wear OS and watchOS

Watch apps get their own surface vocabulary — `WatchApp`, `WatchPage`, `WatchTile`, `WatchGlance`, and `WatchComplication` — under `leptoskit::watch`, with hard layout budgets so a phone tree cannot be pasted onto a watch unnoticed. Content still uses leptoskit elements and Tailwind; the containers are watch-sized.

Android Wear OS (`wearos`) and Apple watchOS (`watchos`) are experimental platforms: config and CLI know them, `leptoskit new --platform wearos|watchos` scaffolds host stubs, and `leptoskit native prepare` writes them. Packaging and device install are not implemented yet. Enable with the `watch`, `wearos`, or `watchos` features.
