---
leptoskit_tui: major
---

# leptoskit_tui: the terminal backend

`leptoskit_tui` draws the same shared element tree in the terminal, for long-lived interactive apps such as editors, dashboards, and chat clients, with keyboard focus, the cursor, mouse input, and scrolling handled by the backend.

Every cell is 8×16 logical pixels, so Yoga layout, Tailwind spacing, and text measurement land on the monospace grid. `leptoskit dev --platform tui` and `leptoskit build --platform tui` run it, and `examples/opencode` is a full coding-agent interface built with it. The terminal's font is the user's, so font sizes and weights become attributes and colors; `Image` and `Vector` draw a placeholder, Rive playback is unsupported, and right-to-left text is not reordered.
