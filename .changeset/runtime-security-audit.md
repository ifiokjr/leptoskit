---
"release": patch
---

# Harden the Wasm runtime and code push against abuse

- A guest that imports the wasm-bindgen externref table shim with a reference-typed signature can no longer forge a function reference. Calling through one used to panic the interpreter, which aborted the macOS app.
- Plugin payload copies now cost fuel, so a guest looping over `submit` and `cancel` runs out of fuel instead of copying megabytes per instruction and freezing the app.
- Calls to the console-warn shim are logged at most 16 times per guest.
- Class strings longer than 4 KiB are parsed but no longer cached, so a guest can't pin hundreds of megabytes in the Tailwind parse cache.
- A synchronous Rust plugin that panics on the macOS UI thread now fails that call, and every later call to that plugin, with a native error instead of aborting the app.
- Code push reads `index.json` with a 2 MiB limit instead of 32 MiB, and `CodePushClient` caps rebuilt and downloaded containers at 32 MiB by default instead of 512 MiB. Custom `Transport`s can override the new `fetch_within` to stop reading at that limit.
- A signed release that the installed app can't run is remembered and isn't downloaded again on every check. A new store build forgets the list.
- iOS caches at most 256 colors and 64 fonts, so animating arbitrary colors or text sizes no longer grows memory without limit.
