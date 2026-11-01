---
"release": minor
---

# Update native screens through signals

A native guest re-ran the whole screen function after every state change and diffed the entire tree, so changing 100 labels in a list of 1,000 cards rebuilt 3,000 elements. Screens can now bind a value to a signal and leave the tree alone.

- **`Element::text_from`** and **`Element::background_from`** bind one element's text or background colour to a closure that reads a `RwSignal`. The element carries the value read when it was built, so the first frame is right and the web renderer is unchanged.
- **`leptoskit::guest::refresh()`** reads the bindings, sends only the nodes whose value differs, and commits them as one transaction. A refresh that finds nothing changed never reaches the host.
- **`leptoskit::prelude`** now exports `RwSignal`, `Get`, and `Set` from `reactive_graph`, so app code writes state the way it does on the web instead of depending on the reactive crate directly.
- **`leptoskit::guest::mount`** gives the application a reactive owner, so a signal created in a scenario, an event handler, or a render lives as long as the guest.

The arena's `mount` scenario now holds its cards in signals. On the Seeker, creating 1,000 cards takes 103 ms instead of 1,438 ms, updating every tenth takes 17 ms instead of 398 ms, and clearing them takes 17 ms instead of 54 ms: faster than Flutter (250 ms, 48 ms, and 28 ms) on all three.
