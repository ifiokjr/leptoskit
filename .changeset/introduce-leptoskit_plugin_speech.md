---
leptoskit_plugin_speech: major
---

# leptoskit_plugin_speech: text-to-speech

`leptoskit_plugin_speech` is the leptoskit plugin for bounded, cancellable text-to-speech with one active utterance per plugin.

Add `leptoskit_plugin_speech` as a direct dependency and select the `speech` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Speech` rather than resolving the plugin on every call. iOS and macOS use `AVSpeechSynthesizer`, Android uses `TextToSpeech`, and browser apps use `web::Speech` over `speechSynthesis`; Windows and Linux have no speech backend yet. Overlapping speech returns `Busy`, and pause, resume, and word-boundary events are not implemented.
