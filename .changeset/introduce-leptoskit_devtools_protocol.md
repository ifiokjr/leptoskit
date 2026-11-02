---
leptoskit_devtools_protocol: breaking
---

# leptoskit_devtools_protocol: the DevTools wire format

`leptoskit_devtools_protocol` holds the versioned serde messages shared by the DevTools agents, hub, interface, and MCP tools.

It has no platform code: the browser, native host, guest, and front end all speak the same protocol. A version mismatch is refused and both versions are named.
