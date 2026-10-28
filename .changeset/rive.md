---
"release": minor
---

# Add browser Rive playback

Add `leptoskit_rive` and the optional `leptoskit::rive` export. Browser players use an app-supplied official runtime, accept bounded app-owned assets, reject external asset loading, disable file-triggered navigation, and release their observers and renderer on drop. Playback respects visibility and reduced motion; canvas allocations are bounded. Native widget rendering is not yet available.
