---
"release": minor
---

# Update lists and conditions without rebuilding the screen

`For` rebuilds its own rows when the collection changes, and `Show` swaps its child when the condition changes. A press that writes a signal — a bound label, a list, a condition — settles that part of the screen. The view runs again only when the view itself read the signal, or when the screen keeps its state in a plain value.

`leptoskit::guest::spawn` runs a task on the runtime's executor, and `leptoskit::guest::poll` is what `leptoskit_poll` calls, so a generated app does not keep a pool of its own. The host still flushes after the poll. A guest built before the flush export keeps working.
