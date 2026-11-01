<h1 align="center">leptoskit_plugin_speech</h1>

<p align="center">
  <strong>leptoskit_plugin_speech</strong> is the leptoskit plugin for bounded, cancellable text-to-speech with one active utterance per plugin.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_speech"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_speech"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_speech.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_speech"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_speech/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

Text-to-speech using AVSpeechSynthesizer on iOS/macOS, Android TextToSpeech, and browser speechSynthesis. Discover voices, speak until completion, stop owned speech, and inspect whether the plugin owns an active utterance.

Add the crate as a direct dependency and `"speech"` to `native.plugins`. Apple registration uses AVFoundation on iOS and macOS. Android requires the package visibility query declared by `android-query-actions` metadata:

```xml
<queries><intent><action android:name="android.intent.action.TTS_SERVICE" /></intent></queries>
```

Browser applications use `web::Speech`. There is no Rust desktop implementation for Windows/Linux yet; these targets must not report speech support.

A plugin accepts one active utterance. Overlapping calls return `Busy`, and `speak` completes only after an engine end callback. `stop`, close, or dropping a guest call cancels active work. Native callbacks marshal to the UI thread and match the original utterance before completing it; late callbacks cannot finish replacement work. Android cancels promptly on its main thread, shuts down its engine on close, and bounds initialization to ten seconds. Browser handles share one active utterance; their stop operation does not cancel unrelated page speech when they own none. Browser futures stop their utterance on drop.

Rate and pitch use integer thousandths, with 1000 normal and a portable range of 500–2000. Volume is 0–1000; zero is muted. Apple rate maps to the platform's normal rate and clamps to its documented range, so exact duration differs by engine. Text is limited to 4096 UTF-8 bytes and 4000 UTF-16 units; Android also enforces its engine limit. Language selection accepts `language[-Script][-Region]`, such as `en-GB` or `zh-Hant-TW`; use a discovered voice ID for other engine-specific language variants. Voice ID and language cannot be selected together. Missing voices/languages report `Unavailable` where the platform exposes failure.

Android voices may use a network engine. Voice discovery exposes local-service status where available; Apple does not expose that flag. Browser voice discovery may initially be empty until its `voiceschanged` event. Browsers can require a user gesture, and permission denials return `PermissionDenied`. This plugin does not change the application's audio session or override system silent modes.

The implementation ports voice discovery, speaking, stopping and active status from [Expo Speech](https://docs.expo.dev/versions/latest/sdk/speech/). Pause/resume, word boundary events, and audio-session configuration are not implemented.

Verification includes binary/error tests, non-audible native lifecycle checks, and a real Chrome test of overlap, stop, future drop and close. The browser test sets volume to zero and uses `--mute-audio`; its local WebDriver configuration permits autoplay only for tests. No audible hardware output has been verified, and a finished engine callback is not proof that a listener heard sound.

## How it fits

Add `leptoskit_plugin_speech` as a direct dependency and select the `speech` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Speech` rather than resolving the plugin on every call. iOS and macOS use `AVSpeechSynthesizer`, Android uses `TextToSpeech`, and the browser uses `speechSynthesis`; overlapping speech returns `Busy`.

## Use it

```rust,ignore
use leptoskit_plugin_speech::{guest::Speech, Utterance};
let speech = Speech::new()?;
let voices = speech.voices().await?;
let request = Utterance::new("Hello")?.with_language("en-GB")?;
speech.speak(&request).await?;
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_speech)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Plugin inventory](https://ifiokjr.github.io/leptoskit/platforms/plugin-inventory.html)
