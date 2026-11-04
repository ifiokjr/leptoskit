---
leptoskit_virtual: major
---

# leptoskit_virtual: headless virtualization

`leptoskit_virtual` is headless list virtualization in the style of TanStack Virtual: fixed or measured sizes, overscan, aligned scroll targets, and a window over the shared renderer.

It decides which items a viewport intersects so a ten-thousand-row list only builds the visible rows, on web and native alike. Feed a `Virtualizer` the viewport size and scroll offset, render the window with `VirtualList` (vertical or horizontal), and hand back real sizes with `measure` as items are laid out.
