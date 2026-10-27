---
"release": minor
---

# Signal hooks for battery, network, location, and clipboard

`leptoskit_plugin_battery`, `leptoskit_plugin_network`, `leptoskit_plugin_location`, and `leptoskit_plugin_clipboard` gain a `hooks` feature with leptos-use-style signal hooks: `use_battery`, `use_network`, `use_geolocation`, and `use_clipboard`. Each hook returns a signal plus one driving future handed to the host executor — nothing is spawned internally. Snapshot hooks publish the first reading, then follow platform changes until the reactive owner is disposed or a platform call fails, leaving the last value in place; the clipboard hook mirrors successful read, write, and clear actions in a signal. Each crate exposes a sealed `Source` trait implemented by the interpreter client and a browser source, with in-crate test doubles covering the loops natively.
