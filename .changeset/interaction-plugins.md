---
"release": minor
---

# Add document import, orientation and sensor capabilities

Add independent `document_picker`, `screen_orientation`, `light_sensor` and `pedometer` crates with selected-plugin native registration and typed binary guest contracts.

Document picker imports one ordinary file through iOS, macOS, Android or a browser, with bounded off-thread native reads, balanced resource access and shared Rust reply storage. Windows/Linux hosts can use an explicit parent-window API. Directory selection, export, multiple files and persistent grants remain separate work.

Screen orientation reports the presented interface and holds a cancellable policy lease for the owning iOS scene, Android Activity or browser document. Cancellation and lifecycle cleanup release the request; a pending lease does not prove that the operating system rotated the interface. Native desktop locking is unavailable.

Light sensor acquires one ambient-illuminance sample on Android or supported browsers. Pedometer preserves the distinction between iOS interval counts and Android cumulative boot counters, supports explicit permission requests, and bounds iOS historical queries. Pedometer purpose strings and Android activity-recognition permission remain application-owned. These ports do not claim physical sensor accuracy or verified access to real activity history.

Extend generated hosts with weak Apple presentation context and an Android activity-result broker that retains presentation ownership through cancellation and rejects stale results. Generated Android Activities handle orientation configuration changes without recreating the guest. Extend browser and Android integration fixtures with synthetic document, light-sensor, orientation and broker lifecycle checks.
