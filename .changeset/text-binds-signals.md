---
"release": minor
---

# Bind values by default, and settle them without a call

`Text` takes a string or a closure, and a closure is a binding: `Text(move || format!("Count: {}", count.get()))` follows the signal it reads, and `background` and the other value props take a closure the same way. Nothing is constructed by hand and nothing needs calling — the native host settles every binding after each turn, through a `leptoskit_flush` export the generated apps carry, so writing a signal is the whole change. A guest built before the export keeps working, because the host looks for it.

`leptoskit::guest::refresh` is now `leptoskit::guest::flush`, and an app should not need either: the guest visits only the nodes that read application state, and only sends the ones whose value differs.

The arena app is an app again. Its header binds the beacon's colour and its status text with `View().background(move || …)` and `Text(move || …)`, its cards bind their titles with `Text`, and its model no longer carries a `settle` for a host call the framework makes for it. The Seeker measures 1,000 cards in 108 ms, an update of every tenth in 19 ms, and clearing them in 17 ms.
