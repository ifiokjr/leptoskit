---
leptoskit_plugin_speech: breaking
---

# leptoskit_plugin_speech: text-to-speech

`leptoskit_plugin_speech` is the leptoskit plugin for bounded, cancellable text-to-speech with one active utterance per plugin.

Add `leptoskit_plugin_speech` as a direct dependency and select the `speech` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Speech` rather than resolving the plugin on every call. iOS and macOS use `AVSpeechSynthesizer`, Android uses `TextToSpeech`, and the browser uses `speechSynthesis`; overlapping speech returns `Busy`.
