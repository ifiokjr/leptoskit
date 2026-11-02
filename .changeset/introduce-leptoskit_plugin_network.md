---
leptoskit_plugin_network: breaking
---

# leptoskit_plugin_network: network state

`leptoskit_plugin_network` is the leptoskit plugin for network state and demand-driven change observation.

Add `leptoskit_plugin_network` as a direct dependency and select the `network` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Network` rather than resolving the plugin on every call. Apple uses `NWPathMonitor`, Android uses `ConnectivityManager`, desktop reports interface state, and the browser uses `navigator.onLine` plus `NetworkInformation`; an active connection never claims a server is reachable.
