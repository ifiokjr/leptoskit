---
"release": patch
---

# Build mobile screens several times faster

Profiling the generated iOS and Android apps while they create, update and clear 1,000 cards found work that grew with the size of the screen, and work done for nothing. All of it is gone:

- **iOS inserts.** Reading `UIView.subviews` copies the whole child list, and each insert read it twice. Inserting 1,000 cards spent over 200 ms there; the renderer now counts children itself.
- **The interpreter.** Native apps ran their WebAssembly with a portable dispatch loop. They now use the interpreter's tail-call dispatch, which runs app code about two and a half times faster.
- **View flattening.** Finding a view's native position summed every earlier sibling, so it grew with each one. Appending now takes constant time: 17 ms to under 1 ms for 1,000 cards.
- **Text measurement.** Layout measured each label three or four times, under different heights it doesn't depend on. Each label is now measured once per width. iOS also reuses its fonts and text attributes instead of building them for every label.
- **Empty renders.** A native app pumps after every plugin reply and display frame, and asked for a render each time, encoding, decoding and applying an empty transaction. `lk_app_render_pending` now says when there is nothing to send, and the generated iOS and Android apps skip those renders. A plugin call with a small payload went from about 19 µs to 9 µs on both.
- **Comparing memory.** WebAssembly can copy memory in bulk but not compare it, so Rust compared byte by byte inside the interpreter. Guests now compare eight bytes at a time, so checking a 64 KiB reply takes 0.05 ms instead of 0.4.
- **A binary render transaction.** The generated iOS and Android apps read each render with `lk_app_render_binary`: fixed-width fields, read with a cursor, instead of JSON that Swift's `JSONDecoder` took 30 ms to decode for 1,000 cards and Android parsed into a tree of objects. It carries exactly what the JSON transaction does, which stays available through `lk_app_render`; properties the renderers apply to every view are typed fields, and any other property travels in its JSON form. A shared fixture holds the Rust encoder, the Swift decoder and the Kotlin decoder to the same bytes.
- **Native views.** Both renderers set a property only when it changes, reuse fonts and colors, and recycle the plain containers and labels a cleared screen leaves behind, as React Native does. Android no longer allocates a background drawable for every view, and keeps a view's layout parameters when its frame is unchanged.

In the arena on the iPhone simulator, creating 1,000 cards went from 583 ms to 150 ms, updating every tenth from 54 ms to 18 ms, and clearing them from 54 ms to 20 ms; a 64 KiB plugin round trip went from 1.8 ms to 0.09 ms. On the Android emulator, creating them went from 550 ms to 77 ms, updating from 100 ms to 18 ms, and clearing from 50 ms to 17 ms.
