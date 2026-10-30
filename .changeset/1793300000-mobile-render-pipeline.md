---
"release": patch
---

# Build mobile screens several times faster

Profiling the generated iOS and Android apps while they create, update and clear 1,000 cards found work that grew with the size of the screen, and work done for nothing. All of it is gone:

- **iOS inserts.** Reading `UIView.subviews` copies the whole child list, and each insert read it twice. Inserting 1,000 cards spent over 200 ms there; the renderer now counts children itself.
- **The interpreter.** Native apps ran their WebAssembly with a portable dispatch loop. They now use the interpreter's tail-call dispatch, which runs app code about two and a half times faster.
- **View flattening.** Finding a view's native position summed every earlier sibling, so it grew with each one. Appending now takes constant time: 17 ms to under 1 ms for 1,000 cards.
- **Text measurement.** Layout measured each label three or four times, under different heights it doesn't depend on. Each label is now measured once per width. iOS also reuses its fonts and text attributes instead of building them for every label.
- **Native views.** Both renderers set a property only when it changes, reuse fonts and colors, and recycle the plain containers and labels a cleared screen leaves behind, as React Native does. Android no longer allocates a background drawable for every view, and keeps a view's layout parameters when its frame is unchanged.

In the arena, creating 1,000 cards went from 583 ms to 167 ms on the iPhone simulator, and from 550 ms to 77 ms on the Android emulator. Updating every tenth card went from 54 ms to 19 ms and from 100 ms to 18 ms. Clearing them went from 54 ms to 32 ms and from 50 ms to 17 ms.
