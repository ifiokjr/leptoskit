---
leptoskit_plugin_arguments: breaking
---

# leptoskit_plugin_arguments: launch arguments

`leptoskit_plugin_arguments` is the leptoskit plugin for immutable launch-argument snapshots with typed, non-exiting clap parsing.

Add `leptoskit_plugin_arguments` as a direct dependency and expose the plugin explicitly from the native host; it has no `native.plugins` entry. Guests enable the `guest` feature and reuse `guest::ArgumentsClient` rather than resolving the plugin on every call. The host chooses which arguments to disclose; mobile and browser process capture returns `Unavailable`, while explicit snapshots and parsing still work.
