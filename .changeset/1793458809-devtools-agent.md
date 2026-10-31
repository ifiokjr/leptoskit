---
"release": minor
---

# Add the DevTools host agent

The `leptoskit_devtools` crate's `HostAgent` opens a session with the local hub, reports snapshots and events, and long-polls for commands. `snapshot` turns a native surface's committed tree into a protocol snapshot.
