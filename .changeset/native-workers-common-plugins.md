---
"release": minor
---

Add app-owned directory factories, retained Rust workers, bounded native job accounting through actual cancellation completion, shared native support discovery and Android manifest contributions. Generate runnable macOS AppKit applications alongside iOS and Android; exhaustive matches on `native::Target` must handle `Macos`.

Add separate haptics, battery, device, secure-store and private-filesystem crates and exercise their supported operations in generated sample applications. Their platform implementations expose explicit unknown/unavailable states and do not claim full Expo or Tauri parity. Running synchronous writes may finish after cancellation.
