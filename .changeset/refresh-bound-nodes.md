---
"release": patch
---

# Refresh only the bound nodes

`guest::refresh` now keeps the ids of the nodes that read application state and visits only those, so a screen of a few thousand elements no longer walks all of them to find the hundred that changed, and it reads each binding without cloning it.

The guide says when a binding pays off: a change that arrives outside an event. After every event the native guest renders the view again, bound or not, so a press still rebuilds the screen; the guide now says so, and says to keep a signal in application state rather than creating one per render, since a signal lives as long as the guest.

The arena's own components are capitalized and carry `#[component]`, and its `leptoskit.toml` denies `component_attribute`, so the rule holds there. Its SSR shell is leptos markup over an HTML document, so `markup` stays `any`.

The Android arena target now refuses to measure a locked or dozing device. One such run recorded a lock screen at a third of the usual speed and reported it as the app's numbers; it now fails with "the device is locked; unlock it before measuring".
