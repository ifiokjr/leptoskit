---
"leptoskit_plugin_power_monitor": minor
"leptoskit_plugin_deep_link": minor
"leptoskit_plugin_updater": minor
"leptoskit_plugin_single_instance": minor
---

# Electron desktop parity, first wave

Desktop hosts get Electron-shaped lifecycle APIs without embedding Chromium: `power_monitor` for suspend/resume/lock/AC events and Windows idle queries, `deep_link` for inbound URL/file activations and association descriptors, `updater` hooks with pluggable backends (separate from code push), and second-launch argument handoff on `single_instance`. Docs map Electron concepts to leptoskit plugins and name the remaining gaps.
