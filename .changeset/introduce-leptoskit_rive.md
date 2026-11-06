---
leptoskit_rive: major
---

# leptoskit_rive: Rive playback

`leptoskit_rive` plays Rive animations that ship inside the app: it checks a `.riv` file before loading it, picks one animation or state machine to run, and sends typed state-machine inputs (booleans, numbers, and triggers) so an animation can react to the interface.

The shared `Rive` component uses it on the web through the `web` feature and on macOS, iOS, Android, Linux, and Windows through the `native` feature; the terminal has no Rive playback. Native playback draws vector scenes only and rejects files that use images, fonts, text, layout, scripts, or audio. It never fetches anything over the network, and it should only load files you trust.
