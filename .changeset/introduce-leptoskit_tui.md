---
leptoskit_tui: breaking
---

# leptoskit_tui: the terminal backend

`leptoskit_tui` commits the same shared element tree into an interactive terminal cell grid, with keyboard focus, the terminal cursor, mouse input, and scrolling owned by the backend.

Every cell is 8×16 logical pixels, so Yoga layout, Tailwind spacing, and text measurement land on the monospace grid. `leptoskit dev/build --platform tui` drives it, and `examples/opencode` is a full coding-agent interface built with it.
