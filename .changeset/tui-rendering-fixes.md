---
"release": patch
---

# Fix terminal wide text, resizes, and long lists

Terminal apps draw correctly in three cases they used to get wrong:

- Colored CJK text and emoji no longer overwrite each other. Each wide glyph now lands in the two cells it occupies.
- Resizing the terminal redraws everything on screen. Text that did not move used to stay blank until it changed.
- A scroll view whose content is taller than 32,767 rows, such as a long `VirtualList`, keeps showing its rows once scrolled.

A long scrolled list also renders faster: rows outside the screen are no longer wrapped or drawn. Rendering 10,000 rows into a 40×20 terminal takes 0.1 ms instead of 3 ms.
