---
"release": patch
---

# Keep the Harbour example on one event loop

The scripted Harbour example now runs query updates and rendering on the same Tokio thread, avoiding intermittent signal-read panics while booking a seat.
