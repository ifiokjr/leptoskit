---
"release": minor
---

# Add a binary runtime plugin bridge

Introduce host-granted, bounded native calls and events with interpreter imports, a cancellable Rust guest client, C/Swift/C++/JNI adapters, and independent crypto, OS-information, and clipboard plugin crates. Add `leptoskit generate app` and `leptoskit generate plugin`, native language linting, and a researched inventory of the remaining Expo, Tauri, and Electron capabilities.

This establishes the runtime infrastructure and initial APIs. It does not yet provide the full upstream catalogs, automatic native registration, or complete generated iOS/Android application hosts.
