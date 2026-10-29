---
"release": patch
---

# Render native screens with fewer interpreter instructions

Native apps run their screens as WebAssembly in an interpreter, so every instruction a render executes costs real time. Counting them for the arena's ticker screen (920 elements, with 300 labels changing every frame) showed three avoidable costs, now gone:

- `.class(...)` built its combined class string with `format!`, which cost more than looking the parsed classes up. It now uses the string as given, or joins two strings directly.
- The parsed-class cache hashed every lookup with SipHash. It now uses a fast word-at-a-time hash, as rustc does for its own maps.
- The guest kept its tree in a `BTreeMap` of large nodes, so every render paid a tree lookup and shifted entries on each insert. It now uses a reserved hash map, plus a list of ids in build order, so the host still receives parents before children in a deterministic order.

`.children(...)` also reserves room for its children up front, rather than growing and moving its list.

A ticker frame went from 16.4 million interpreter instructions to 10.1 million, and a render with nothing changed from 13.6 million to 7.0 million. In the arena on macOS, updating every tenth of 1,000 cards went from about 51 ms to 32 ms, and the ticker from about 150 to about 170 updates in five seconds. Rendered output is unchanged.
