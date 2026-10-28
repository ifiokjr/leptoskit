---
"release": patch
---

# Release VoiceOver when a UIKit view stops being modal

A view that dropped `aria-modal` kept UIKit's `accessibilityViewIsModal` set, so VoiceOver went on ignoring the rest of the screen after a dialog closed or stopped being modal. Every update now sets the flag from the current state.
