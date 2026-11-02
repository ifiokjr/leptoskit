---
leptoskit_plugin_devtools: breaking
---

# leptoskit_plugin_devtools: the guest-to-host DevTools bridge

`leptoskit_plugin_devtools` is the bridge plugin that carries a native guest's DevTools data to the host that already reports the tree.

Apps never select it. The shared runtime installs it in `leptoskit dev` builds, and the host merges the guest's route, state, forms, and queries into the session it reports.
