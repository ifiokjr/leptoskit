---
leptoskit_rive: breaking
---

# leptoskit_rive: Rive playback

`leptoskit_rive` loads app-owned `.riv` assets with typed state-machine inputs and an owned player for browser canvas and native vector playback.

The shared `Rive` component uses it on every backend that supports vector playback; the runtime crate pins the offline C++ build behind it. Apps bring their own assets and keep playback out of the network path.
