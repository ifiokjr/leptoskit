---
"release": minor
---

# Add leptoskit_aria: headless accessible components

New `leptoskit_aria` crate with the widget-behavior layer over the declarative ARIA model: `DisclosureButton`/`DisclosurePanel`, `ToggleButton` (including mixed state), `TabList`/`Tab`/`TabPanel` with roving focus, `Field` label/help/error wiring for any input, and `Dialog`/`DialogTitle`/`DialogDescription`. Components are plain functions of state plus callbacks emitting the shared element tree, so one widget renders with correct semantics on the web and every native backend, and the renderer's validation rejects half-wired trees. Re-exported behind the `leptoskit` `aria` feature and documented in the new accessible-components guide.
