---
"release": minor
---

# Add leptoskit_aria: headless accessible components

New `leptoskit_aria` crate with the widget-behavior layer over the declarative ARIA model: `DisclosureButton`/`DisclosurePanel`, `ToggleButton` (including mixed state), `Tabs` building a tab list, tabs, and panels whose selection and set position cannot disagree, `Field` label/help/error wiring for any input, and `Dialog` building a container named by its title and described by its description. Components emit the shared element tree, so one widget renders with correct semantics on the web and every native backend, and the renderer's validation rejects half-wired trees. Tabs stay in the tab order and dialogs are not marked modal until the renderer gains key events and focus containment. Re-exported behind the `leptoskit` `aria` feature and documented in the new accessible-components guide.
